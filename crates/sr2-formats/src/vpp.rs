//! `.vpp_pc` archive container.
//!
//! Only the header magic/version are confirmed so far (observed in the shipped
//! archives); the rest of the layout will be filled in from the exe's loader.

/// First u32 of every shipped `.vpp_pc`.
pub const MAGIC: u32 = 0x5189_0ACE;
/// Second u32 in every shipped `.vpp_pc`.
pub const VERSION: u32 = 4;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    TooShort,
    BadMagic(u32),
    UnsupportedVersion(u32),
}

/// Validate the archive preamble.
pub fn check_header(data: &[u8]) -> Result<(), Error> {
    let word = |i: usize| -> Result<u32, Error> {
        data.get(i..i + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
            .ok_or(Error::TooShort)
    };
    match (word(0)?, word(4)?) {
        (MAGIC, VERSION) => Ok(()),
        (MAGIC, v) => Err(Error::UnsupportedVersion(v)),
        (m, _) => Err(Error::BadMagic(m)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_v4_header() {
        let mut h = [0u8; 8];
        h[..4].copy_from_slice(&MAGIC.to_le_bytes());
        h[4..].copy_from_slice(&VERSION.to_le_bytes());
        assert_eq!(check_header(&h), Ok(()));
        assert_eq!(check_header(&h[..6]), Err(Error::TooShort));
    }
}
