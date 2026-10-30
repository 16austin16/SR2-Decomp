//! `.vpp_pc` archive container (version 4, as shipped with SR2 PC).
//!
//! Layout, derived from the shipped archives (all 18 parse; see `examples/vpp.rs`):
//!
//! | offset  | contents |
//! |---------|----------|
//! | 0x000   | [`RawHeader`] (magic, version, …), padded to 0x800 |
//! | 0x800   | `entry_count` × 28-byte [`RawEntry`], padded to 0x800 |
//! | …       | filename string table (`names_size` bytes, NUL-separated), padded to 0x800 |
//! | …       | extension string table (`extensions_size` bytes), padded to 0x800 |
//! | …       | file data; each entry's `data_offset` is relative to this point |
//!
//! Fields marked `unknown`/`runtime_*` are zero or 0xFFFFFFFF on disk and are
//! presumably filled in by the game's loader. That is still to be confirmed in the exe.

use std::io::{self, Read, Seek, SeekFrom};

/// First u32 of every shipped `.vpp_pc`.
pub const MAGIC: u32 = 0x5189_0ACE;
/// Second u32 in every shipped `.vpp_pc`.
pub const VERSION: u32 = 4;
/// Section alignment within the archive.
pub const ALIGN: u64 = 0x800;
/// Size of one directory entry.
pub const ENTRY_SIZE: usize = 28;

const HEADER_SIZE: usize = 0x180;
const NOT_COMPRESSED: u32 = 0xFFFF_FFFF;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    TooShort,
    BadMagic(u32),
    UnsupportedVersion(u32),
    /// Archive-level or per-entry compression; not seen in the shipped PC data.
    Compressed,
    BadStringOffset(u32),
    EntryOutOfBounds { index: usize },
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::UnexpectedEof { Error::TooShort } else { Error::Io(e) }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}

/// On-disk header fields (offsets from the start of the file).
#[derive(Debug, Clone)]
pub struct RawHeader {
    /// 0x14C. 0 or 2 in shipped archives; meaning unknown.
    pub flags: u32,
    /// 0x154
    pub entry_count: u32,
    /// 0x158. Equals the file size.
    pub archive_size: u32,
    /// 0x15C. `entry_count * 28`
    pub directory_size: u32,
    /// 0x160
    pub names_size: u32,
    /// 0x164
    pub extensions_size: u32,
    /// 0x168
    pub data_size: u32,
    /// 0x16C. 0xFFFFFFFF = not compressed
    pub compressed_data_size: u32,
}

/// On-disk directory entry.
#[derive(Debug, Clone, Copy)]
pub struct RawEntry {
    pub name_offset: u32,
    pub extension_offset: u32,
    pub unknown_08: u32,
    pub data_offset: u32,
    pub size: u32,
    /// 0xFFFFFFFF = stored uncompressed.
    pub compressed_size: u32,
    pub unknown_18: u32,
}

#[derive(Debug, Clone)]
pub struct Entry {
    /// Base name without extension, e.g. `customization_items`.
    pub name: String,
    /// Extension without the dot, e.g. `xtbl`.
    pub extension: String,
    /// Absolute file offset of the data.
    pub offset: u64,
    pub size: u32,
    pub raw: RawEntry,
}

impl Entry {
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.name, self.extension)
    }
}

#[derive(Debug, Clone)]
pub struct Archive {
    pub header: RawHeader,
    pub entries: Vec<Entry>,
    /// Absolute offset of the data section.
    pub data_start: u64,
}

fn align(v: u64) -> u64 {
    v.div_ceil(ALIGN) * ALIGN
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn read_vec<R: Read>(r: &mut R, len: usize) -> Result<Vec<u8>, Error> {
    let mut v = vec![0; len];
    r.read_exact(&mut v)?;
    Ok(v)
}

fn c_string(table: &[u8], offset: u32) -> Result<String, Error> {
    let start = offset as usize;
    let rest = table.get(start..).ok_or(Error::BadStringOffset(offset))?;
    let end = rest.iter().position(|&c| c == 0).ok_or(Error::BadStringOffset(offset))?;
    Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
}

/// Validate the archive preamble.
pub fn check_header(data: &[u8]) -> Result<(), Error> {
    if data.len() < 8 {
        return Err(Error::TooShort);
    }
    match (u32_at(data, 0), u32_at(data, 4)) {
        (MAGIC, VERSION) => Ok(()),
        (MAGIC, v) => Err(Error::UnsupportedVersion(v)),
        (m, _) => Err(Error::BadMagic(m)),
    }
}

impl Archive {
    /// Parse the header and directory. File data is not read.
    pub fn read<R: Read + Seek>(r: &mut R) -> Result<Self, Error> {
        r.seek(SeekFrom::Start(0))?;
        let h = read_vec(r, HEADER_SIZE)?;
        check_header(&h)?;
        let header = RawHeader {
            flags: u32_at(&h, 0x14C),
            entry_count: u32_at(&h, 0x154),
            archive_size: u32_at(&h, 0x158),
            directory_size: u32_at(&h, 0x15C),
            names_size: u32_at(&h, 0x160),
            extensions_size: u32_at(&h, 0x164),
            data_size: u32_at(&h, 0x168),
            compressed_data_size: u32_at(&h, 0x16C),
        };
        if header.compressed_data_size != NOT_COMPRESSED {
            return Err(Error::Compressed);
        }

        let dir_start = ALIGN;
        let names_start = align(dir_start + header.directory_size as u64);
        let ext_start = align(names_start + header.names_size as u64);
        let data_start = align(ext_start + header.extensions_size as u64);

        r.seek(SeekFrom::Start(dir_start))?;
        let dir = read_vec(r, header.entry_count as usize * ENTRY_SIZE)?;
        r.seek(SeekFrom::Start(names_start))?;
        let names = read_vec(r, header.names_size as usize)?;
        r.seek(SeekFrom::Start(ext_start))?;
        let exts = read_vec(r, header.extensions_size as usize)?;

        let entries = dir
            .chunks_exact(ENTRY_SIZE)
            .enumerate()
            .map(|(index, e)| {
                let raw = RawEntry {
                    name_offset: u32_at(e, 0),
                    extension_offset: u32_at(e, 4),
                    unknown_08: u32_at(e, 8),
                    data_offset: u32_at(e, 12),
                    size: u32_at(e, 16),
                    compressed_size: u32_at(e, 20),
                    unknown_18: u32_at(e, 24),
                };
                if raw.compressed_size != NOT_COMPRESSED {
                    return Err(Error::Compressed);
                }
                if raw.data_offset as u64 + raw.size as u64 > header.data_size as u64 {
                    return Err(Error::EntryOutOfBounds { index });
                }
                Ok(Entry {
                    name: c_string(&names, raw.name_offset)?,
                    extension: c_string(&exts, raw.extension_offset)?,
                    offset: data_start + raw.data_offset as u64,
                    size: raw.size,
                    raw,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Archive { header, entries, data_start })
    }

    pub fn find(&self, file_name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.file_name().eq_ignore_ascii_case(file_name))
    }

    /// Read one entry's bytes.
    pub fn read_entry<R: Read + Seek>(&self, r: &mut R, entry: &Entry) -> Result<Vec<u8>, Error> {
        r.seek(SeekFrom::Start(entry.offset))?;
        read_vec(r, entry.size as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Build a minimal archive with the same layout as the shipped ones.
    fn build(files: &[(&str, &str, &[u8])]) -> Vec<u8> {
        let mut names = Vec::new();
        let mut exts = Vec::new();
        let mut dir = Vec::new();
        let mut data = Vec::new();
        for (n, x, bytes) in files {
            let name_off = names.len() as u32;
            names.extend_from_slice(n.as_bytes());
            names.push(0);
            let ext_off = exts.len() as u32;
            exts.extend_from_slice(x.as_bytes());
            exts.push(0);
            while data.len() % 16 != 0 {
                data.push(0);
            }
            for v in [name_off, ext_off, 0, data.len() as u32, bytes.len() as u32, NOT_COMPRESSED, 0] {
                dir.extend_from_slice(&v.to_le_bytes());
            }
            data.extend_from_slice(bytes);
        }
        let mut out = vec![0u8; ALIGN as usize];
        let put = |out: &mut Vec<u8>, o: usize, v: u32| out[o..o + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut out, 0, MAGIC);
        put(&mut out, 4, VERSION);
        put(&mut out, 0x14C, 2);
        put(&mut out, 0x154, files.len() as u32);
        put(&mut out, 0x15C, dir.len() as u32);
        put(&mut out, 0x160, names.len() as u32);
        put(&mut out, 0x164, exts.len() as u32);
        put(&mut out, 0x168, data.len() as u32);
        put(&mut out, 0x16C, NOT_COMPRESSED);
        for section in [&dir, &names, &exts] {
            out.extend_from_slice(section);
            out.resize(align(out.len() as u64) as usize, 0);
        }
        out.extend_from_slice(&data);
        let size = out.len() as u32;
        put(&mut out, 0x158, size);
        out
    }

    #[test]
    fn accepts_v4_header() {
        let mut h = [0u8; 8];
        h[..4].copy_from_slice(&MAGIC.to_le_bytes());
        h[4..].copy_from_slice(&VERSION.to_le_bytes());
        assert!(check_header(&h).is_ok());
        assert!(matches!(check_header(&h[..6]), Err(Error::TooShort)));
    }

    #[test]
    fn parses_and_reads_entries() {
        let bytes = build(&[("a", "xtbl", b"<root/>"), ("b_long_name", "lua", b"print(1)")]);
        let mut cur = Cursor::new(bytes);
        let vpp = Archive::read(&mut cur).unwrap();
        assert_eq!(vpp.entries.len(), 2);
        let b = vpp.find("B_LONG_NAME.lua").unwrap();
        assert_eq!(vpp.read_entry(&mut cur, b).unwrap(), b"print(1)");
        assert_eq!(vpp.entries[0].file_name(), "a.xtbl");
    }
}
