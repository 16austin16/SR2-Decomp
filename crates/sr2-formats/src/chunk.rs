//! `.chunk_pc` world chunks (the cpu half; `.g_chunk_pc` holds the gpu data).
//!
//! Layout, from `chunk_load_from_memory` (0x4AF280, see docs/analysis/05_streamer.md):
//! - u32 words at the start: [0] magic 0xBBCACA12, [1] version (the game accepts
//!   only 0x79), [3] flags, [4] section count, and header fields the loader copies
//!   into the chunk record ([0x25] object count, [0x35..0x3A] bounding box).
//! - The section table is at the end of the file: `count` entries of 12 bytes
//!   `{kind, unknown, offset}`, with `offset` relative to the start of the file.
//!   The loader dispatches on `kind` (0..=0x26).

pub const MAGIC: u32 = 0xBBCA_CA12;
pub const VERSION: u32 = 0x79;

#[derive(Debug)]
pub enum Error {
    TooShort,
    BadMagic(u32),
    BadSectionTable { count: u32, len: usize },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    pub kind: u32,
    pub unknown: u32,
    pub offset: u32,
    /// Bytes up to the next section (or the section table), so an upper bound.
    pub size: u32,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub version: u32,
    pub flags: u32,
    /// Header word 0x25: number of 0x50-byte objects.
    pub object_count: u32,
    /// Header words 0x35..0x3A: min x/y/z, max x/y/z.
    pub bounds: [f32; 6],
    pub sections: Vec<Section>,
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

impl Chunk {
    pub fn parse(data: &[u8]) -> Result<Chunk, Error> {
        let word = |i: usize| u32_at(data, i * 4).ok_or(Error::TooShort);
        let magic = word(0)?;
        if magic != MAGIC {
            return Err(Error::BadMagic(magic));
        }
        let count = word(4)?;
        let table_len = count as usize * 12;
        if table_len > data.len() {
            return Err(Error::BadSectionTable { count, len: data.len() });
        }
        let table = data.len() - table_len;
        let mut sections: Vec<Section> = (0..count as usize)
            .map(|i| {
                let o = table + i * 12;
                Section {
                    kind: u32_at(data, o).unwrap(),
                    unknown: u32_at(data, o + 4).unwrap(),
                    offset: u32_at(data, o + 8).unwrap(),
                    size: 0,
                }
            })
            .collect();
        if sections.iter().any(|s| s.offset as usize > table) {
            return Err(Error::BadSectionTable { count, len: data.len() });
        }
        let mut ends: Vec<u32> = sections.iter().map(|s| s.offset).collect();
        ends.push(table as u32);
        ends.sort_unstable();
        for s in &mut sections {
            let next = ends.iter().copied().find(|&e| e > s.offset).unwrap_or(table as u32);
            s.size = next - s.offset;
        }
        let mut bounds = [0f32; 6];
        for (i, b) in bounds.iter_mut().enumerate() {
            *b = f32::from_bits(word(0x35 + i)?);
        }
        Ok(Chunk {
            version: word(1)?,
            flags: word(3)?,
            object_count: word(0x25)?,
            bounds,
            sections,
        })
    }
}

/// Printable ASCII runs of at least `min` bytes, NUL- or edge-terminated.
pub fn strings(data: &[u8], min: usize) -> Vec<String> {
    data.split(|&c| !(0x20..0x7F).contains(&c))
        .filter(|s| s.len() >= min)
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(sections: &[(u32, &[u8])]) -> Vec<u8> {
        let mut data = vec![0u8; 0x100];
        data[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        data[4..8].copy_from_slice(&VERSION.to_le_bytes());
        data[16..20].copy_from_slice(&(sections.len() as u32).to_le_bytes());
        let mut table = Vec::new();
        for (kind, body) in sections {
            table.extend_from_slice(&kind.to_le_bytes());
            table.extend_from_slice(&0u32.to_le_bytes());
            table.extend_from_slice(&(data.len() as u32).to_le_bytes());
            data.extend_from_slice(body);
        }
        data.extend_from_slice(&table);
        data
    }

    #[test]
    fn parses_section_table_at_end() {
        let data = build(&[(5, b"abcd"), (0x26, b"tree\0\0\0\0")]);
        let c = Chunk::parse(&data).unwrap();
        assert_eq!(c.version, VERSION);
        assert_eq!(c.sections.len(), 2);
        assert_eq!(
            (c.sections[0].kind, c.sections[0].offset, c.sections[0].size),
            (5, 0x100, 4)
        );
        assert_eq!(
            (c.sections[1].kind, c.sections[1].offset, c.sections[1].size),
            (0x26, 0x104, 8)
        );
    }

    #[test]
    fn rejects_bad_magic() {
        let mut data = build(&[]);
        data[0] = 0;
        assert!(matches!(Chunk::parse(&data), Err(Error::BadMagic(_))));
    }

    #[test]
    fn finds_strings() {
        assert_eq!(
            strings(b"\x01name_one\0x\0two_names\0", 4),
            vec!["name_one", "two_names"]
        );
    }
}
