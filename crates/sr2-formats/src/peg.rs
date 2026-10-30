//! Texture containers: `.peg_pc` (header) + `.g_peg_pc` (pixel data), version 10.
//!
//! Layout, derived from the shipped files (`examples/peg.rs check` parses all of them):
//!
//! | offset | contents |
//! |--------|----------|
//! | 0x00   | [`Header`]: magic `GEKV`, version 10, sizes, counts (0x18 bytes) |
//! | 0x18   | `entry_count` × 0x30-byte [`Bitmap`] |
//! | …      | `bitmap_count` NUL-terminated names (`foo.tga`) |
//!
//! Each bitmap's pixels are at `offset` in the `.g_peg_pc`, `frame_size` bytes,
//! all mip levels back to back. The whole `.g_peg_pc` is `data_size` bytes.
//! Fields marked `unknown_*` are kept verbatim so files round-trip.

use std::fmt;

pub const MAGIC: u32 = 0x564B_4547; // "GEKV"
pub const VERSION: u16 = 10;
const HEADER_SIZE: usize = 0x18;
const BITMAP_SIZE: usize = 0x30;

/// Bitmap pixel formats seen in shipped data (D3D-style numbering, offset by 400).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Dxt1,
    Dxt3,
    Dxt5,
    R5G6B5,
    A1R5G5B5,
    A4R4G4B4,
    R8G8B8,
    /// Stored little-endian, so the bytes are B, G, R, A.
    A8R8G8B8,
    Other(u16),
}

impl Format {
    pub fn from_raw(v: u16) -> Self {
        match v {
            400 => Self::Dxt1,
            401 => Self::Dxt3,
            402 => Self::Dxt5,
            403 => Self::R5G6B5,
            404 => Self::A1R5G5B5,
            405 => Self::A4R4G4B4,
            406 => Self::R8G8B8,
            407 => Self::A8R8G8B8,
            v => Self::Other(v),
        }
    }

    pub fn raw(self) -> u16 {
        match self {
            Self::Dxt1 => 400,
            Self::Dxt3 => 401,
            Self::Dxt5 => 402,
            Self::R5G6B5 => 403,
            Self::A1R5G5B5 => 404,
            Self::A4R4G4B4 => 405,
            Self::R8G8B8 => 406,
            Self::A8R8G8B8 => 407,
            Self::Other(v) => v,
        }
    }

    /// Bytes for one mip level of `w`×`h`, or `None` if unknown.
    pub fn level_size(self, w: u32, h: u32) -> Option<usize> {
        let blocks = || (w.div_ceil(4) * h.div_ceil(4)) as usize;
        Some(match self {
            Self::Dxt1 => blocks() * 8,
            Self::Dxt3 | Self::Dxt5 => blocks() * 16,
            Self::R5G6B5 | Self::A1R5G5B5 | Self::A4R4G4B4 => (w * h * 2) as usize,
            Self::R8G8B8 => (w * h * 3) as usize,
            Self::A8R8G8B8 => (w * h * 4) as usize,
            Self::Other(_) => return None,
        })
    }
}

#[derive(Debug)]
pub enum Error {
    TooShort,
    BadMagic(u32),
    UnsupportedVersion(u16),
    BadName,
    DataOutOfBounds { index: usize },
    UnsupportedFormat(Format),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone)]
pub struct Header {
    /// 0x06. Always 0 in shipped files.
    pub unknown_06: u16,
    /// 0x0C. Size of the matching `.g_peg_pc`.
    pub data_size: u32,
    /// 0x12. Always 0 in shipped files.
    pub unknown_12: u16,
    /// 0x16. 16 in shipped files; presumably data alignment.
    pub align: u16,
}

#[derive(Debug, Clone)]
pub struct Bitmap {
    /// 0x00. Offset of the pixel data in the `.g_peg_pc`.
    pub offset: u32,
    pub width: u16,
    pub height: u16,
    pub format: Format,
    /// 0x0A..0x10: palette format and animation tile sizes (0 in the files seen so far).
    pub unknown_0a: [u8; 6],
    /// 0x10.
    pub frames: u16,
    /// 0x12.
    pub flags: u16,
    /// 0x14..0x1A: runtime name pointer and palette size (0 on disk).
    pub unknown_14: [u8; 6],
    /// 0x1A.
    pub fps: u8,
    /// 0x1B.
    pub mip_levels: u8,
    /// 0x1C. Bytes of pixel data for all mip levels.
    pub frame_size: u32,
    /// 0x20..0x30: runtime pointers (0 on disk).
    pub unknown_20: [u8; 16],
}

#[derive(Debug, Clone)]
pub struct Peg {
    pub header: Header,
    /// One per frame. For non-animated pegs, one per bitmap.
    pub bitmaps: Vec<Bitmap>,
    pub names: Vec<String>,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

impl Peg {
    /// Parse a `.peg_pc` header file.
    pub fn parse(b: &[u8]) -> Result<Self, Error> {
        if b.len() < HEADER_SIZE {
            return Err(Error::TooShort);
        }
        let magic = u32_at(b, 0);
        if magic != MAGIC {
            return Err(Error::BadMagic(magic));
        }
        let version = u16_at(b, 4);
        if version != VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let header =
            Header { unknown_06: u16_at(b, 6), data_size: u32_at(b, 0xC), unknown_12: u16_at(b, 0x12), align: u16_at(b, 0x16) };
        let bitmap_count = u16_at(b, 0x10) as usize;
        let entry_count = u16_at(b, 0x14) as usize;
        let names_at = HEADER_SIZE + entry_count * BITMAP_SIZE;
        if b.len() < names_at {
            return Err(Error::TooShort);
        }
        let bitmaps = (0..entry_count)
            .map(|i| {
                let e = &b[HEADER_SIZE + i * BITMAP_SIZE..][..BITMAP_SIZE];
                Bitmap {
                    offset: u32_at(e, 0),
                    width: u16_at(e, 4),
                    height: u16_at(e, 6),
                    format: Format::from_raw(u16_at(e, 8)),
                    unknown_0a: e[0x0A..0x10].try_into().unwrap(),
                    frames: u16_at(e, 0x10),
                    flags: u16_at(e, 0x12),
                    unknown_14: e[0x14..0x1A].try_into().unwrap(),
                    fps: e[0x1A],
                    mip_levels: e[0x1B],
                    frame_size: u32_at(e, 0x1C),
                    unknown_20: e[0x20..0x30].try_into().unwrap(),
                }
            })
            .collect();
        let mut names = Vec::with_capacity(bitmap_count);
        let mut rest = &b[names_at..];
        for _ in 0..bitmap_count {
            let end = rest.iter().position(|&c| c == 0).ok_or(Error::BadName)?;
            names.push(String::from_utf8(rest[..end].to_vec()).map_err(|_| Error::BadName)?);
            rest = &rest[end + 1..];
        }
        Ok(Peg { header, bitmaps, names })
    }

    /// Serialize back to `.peg_pc` bytes. `data_size` is written as stored.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        let names_len: usize = self.names.iter().map(|n| n.len() + 1).sum();
        let total = HEADER_SIZE + self.bitmaps.len() * BITMAP_SIZE + names_len;
        b.extend_from_slice(&MAGIC.to_le_bytes());
        b.extend_from_slice(&VERSION.to_le_bytes());
        b.extend_from_slice(&self.header.unknown_06.to_le_bytes());
        b.extend_from_slice(&(total as u32).to_le_bytes());
        b.extend_from_slice(&self.header.data_size.to_le_bytes());
        b.extend_from_slice(&(self.names.len() as u16).to_le_bytes());
        b.extend_from_slice(&self.header.unknown_12.to_le_bytes());
        b.extend_from_slice(&(self.bitmaps.len() as u16).to_le_bytes());
        b.extend_from_slice(&self.header.align.to_le_bytes());
        for e in &self.bitmaps {
            b.extend_from_slice(&e.offset.to_le_bytes());
            b.extend_from_slice(&e.width.to_le_bytes());
            b.extend_from_slice(&e.height.to_le_bytes());
            b.extend_from_slice(&e.format.raw().to_le_bytes());
            b.extend_from_slice(&e.unknown_0a);
            b.extend_from_slice(&e.frames.to_le_bytes());
            b.extend_from_slice(&e.flags.to_le_bytes());
            b.extend_from_slice(&e.unknown_14);
            b.push(e.fps);
            b.push(e.mip_levels);
            b.extend_from_slice(&e.frame_size.to_le_bytes());
            b.extend_from_slice(&e.unknown_20);
        }
        for n in &self.names {
            b.extend_from_slice(n.as_bytes());
            b.push(0);
        }
        b
    }

    /// The pixel data of bitmap `i` (all mip levels) within `data` (the `.g_peg_pc`).
    pub fn bitmap_data<'a>(&self, i: usize, data: &'a [u8]) -> Result<&'a [u8], Error> {
        let e = &self.bitmaps[i];
        let (start, len) = (e.offset as usize, e.frame_size as usize);
        data.get(start..start + len).ok_or(Error::DataOutOfBounds { index: i })
    }
}

/// Decode the top mip level of a bitmap to RGBA8 (row-major, `width*height*4`).
pub fn decode_rgba(e: &Bitmap, data: &[u8]) -> Result<Vec<u8>, Error> {
    let (w, h) = (e.width as usize, e.height as usize);
    let need = e.format.level_size(w as u32, h as u32).ok_or(Error::UnsupportedFormat(e.format))?;
    let data = data.get(..need).ok_or(Error::TooShort)?;
    let mut out = vec![0u8; w * h * 4];
    match e.format {
        Format::A8R8G8B8 => {
            for (o, i) in out.chunks_exact_mut(4).zip(data.chunks_exact(4)) {
                o.copy_from_slice(&[i[2], i[1], i[0], i[3]]);
            }
        }
        Format::Dxt1 | Format::Dxt3 | Format::Dxt5 => {
            let bsize = if e.format == Format::Dxt1 { 8 } else { 16 };
            let bw = w.div_ceil(4);
            for (bi, blk) in data.chunks_exact(bsize).enumerate() {
                let (bx, by) = (bi % bw * 4, bi / bw * 4);
                let px = decode_block(e.format, blk);
                for (k, p) in px.iter().enumerate() {
                    let (x, y) = (bx + k % 4, by + k / 4);
                    if x < w && y < h {
                        out[(y * w + x) * 4..][..4].copy_from_slice(p);
                    }
                }
            }
        }
        f => return Err(Error::UnsupportedFormat(f)),
    }
    Ok(out)
}

/// Encode RGBA8 into A8R8G8B8 bitmap data (one mip level).
pub fn encode_a8r8g8b8(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect()
}

fn rgb565(c: u16) -> [u32; 3] {
    let (r, g, b) = ((c >> 11) as u32 & 31, (c >> 5) as u32 & 63, c as u32 & 31);
    [(r << 3) | (r >> 2), (g << 2) | (g >> 4), (b << 3) | (b >> 2)]
}

fn decode_block(fmt: Format, blk: &[u8]) -> [[u8; 4]; 16] {
    let col = if fmt == Format::Dxt1 { blk } else { &blk[8..] };
    let (c0, c1) = (u16_at(col, 0), u16_at(col, 2));
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |wa: u32, wb: u32, d: u32| [0, 1, 2].map(|i| ((a[i] * wa + b[i] * wb) / d) as u8);
    let to8 = |c: [u32; 3]| c.map(|v| v as u8);
    let four = fmt != Format::Dxt1 || c0 > c1;
    let palette: [[u8; 4]; 4] = if four {
        let (p2, p3) = (mix(2, 1, 3), mix(1, 2, 3));
        [to8(a), to8(b), p2, p3].map(|c| [c[0], c[1], c[2], 255])
    } else {
        let p2 = mix(1, 1, 2);
        [[to8(a)[0], to8(a)[1], to8(a)[2], 255], [to8(b)[0], to8(b)[1], to8(b)[2], 255], [p2[0], p2[1], p2[2], 255], [0; 4]]
    };
    let idx = u32_at(col, 4);
    let mut px = [[0u8; 4]; 16];
    for (k, p) in px.iter_mut().enumerate() {
        *p = palette[(idx >> (2 * k)) as usize & 3];
    }
    match fmt {
        Format::Dxt3 => {
            let bits = u64::from_le_bytes(blk[..8].try_into().unwrap());
            for (k, p) in px.iter_mut().enumerate() {
                p[3] = ((bits >> (4 * k)) & 15) as u8 * 17;
            }
        }
        Format::Dxt5 => {
            let (a0, a1) = (blk[0] as u32, blk[1] as u32);
            let mut ap = [a0, a1, 0, 0, 0, 0, 0, 0];
            if a0 > a1 {
                for i in 1..7 {
                    ap[i + 1] = ((7 - i as u32) * a0 + i as u32 * a1) / 7;
                }
            } else {
                for i in 1..5 {
                    ap[i + 1] = ((5 - i as u32) * a0 + i as u32 * a1) / 5;
                }
                ap[7] = 255;
            }
            let bits = blk[2..8].iter().rev().fold(0u64, |acc, &x| (acc << 8) | x as u64);
            for (k, p) in px.iter_mut().enumerate() {
                p[3] = ap[(bits >> (3 * k)) as usize & 7] as u8;
            }
        }
        _ => {}
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_header() {
        // ui_mm_load_sr2_logo.peg_pc as shipped.
        let mut b = vec![0u8; 0x48];
        b[..0x18].copy_from_slice(&[
            0x47, 0x45, 0x4B, 0x56, 0x0A, 0, 0, 0, 0x5F, 0, 0, 0, 0, 0, 2, 0, 1, 0, 0, 0, 1, 0, 0x10, 0,
        ]);
        b[0x1C..0x20].copy_from_slice(&[0, 2, 0, 1]);
        b[0x20..0x22].copy_from_slice(&401u16.to_le_bytes());
        b[0x28..0x2C].copy_from_slice(&[1, 0, 1, 0]);
        b[0x32] = 1;
        b[0x33] = 1;
        b[0x34..0x38].copy_from_slice(&0x20000u32.to_le_bytes());
        b.extend_from_slice(b"ui_bms_mm_sr2_logo.tga\0");
        let p = Peg::parse(&b).unwrap();
        assert_eq!((p.bitmaps[0].width, p.bitmaps[0].height, p.bitmaps[0].format), (512, 256, Format::Dxt3));
        assert_eq!(p.names, ["ui_bms_mm_sr2_logo.tga"]);
        assert_eq!(p.to_bytes(), b);
    }
}
