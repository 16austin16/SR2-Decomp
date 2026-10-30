//! Hashes used by the game's lookup tables.

/// Case-insensitive filename hash used by the packfile lookup table
/// (original: `hash_filename_ci` @ 0xBF2BD0).
///
/// `h = rotl(h, 6) ^ c` for each byte. Only ASCII `A`–`Z` are lowercased, and
/// bytes are sign-extended (`movsx`), so bytes >= 0x80 contribute `0xFFFFFFxx`.
/// The original returns `0xFFFFFFFF` for a NULL pointer; see [`filename_hash_ptr`].
pub fn filename_hash(name: &[u8]) -> u32 {
    let mut h = 0u32;
    for &b in name {
        if b == 0 {
            break;
        }
        let mut c = b as i8 as i32;
        if (c - 0x41) as u32 <= 0x19 {
            c += 0x20;
        }
        h = h.rotate_left(6) ^ c as u32;
    }
    h
}

/// NULL-tolerant form matching the original's contract for C strings.
///
/// # Safety
/// `name` must be NULL or point to a NUL-terminated string.
pub unsafe fn filename_hash_ptr(name: *const u8) -> u32 {
    if name.is_null() {
        return 0xFFFF_FFFF;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(name.cast()) };
    filename_hash(s.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Straight transcription of the original asm loop, as a cross-check.
    fn reference(s: &[u8]) -> u32 {
        let mut eax: u32 = 0;
        for &b in s {
            let mut ecx = b as i8 as i32 as u32;
            if ecx.wrapping_sub(0x41) <= 0x19 {
                ecx = ecx.wrapping_add(0x20);
            }
            eax = eax.rotate_left(6) ^ ecx;
        }
        eax
    }

    #[test]
    fn matches_reference_and_is_case_insensitive() {
        for s in [&b""[..], b"a", b"patch.vpp_pc", b"CUSTOMIZATION_ITEMS.XTBL", b"\xe9t\xffZ[@`{"] {
            assert_eq!(filename_hash(s), reference(s), "{s:?}");
        }
        assert_eq!(filename_hash(b"Customization_Items.xtbl"), filename_hash(b"customization_items.xtbl"));
        assert_eq!(filename_hash(b"a"), 0x61);
        assert_eq!(filename_hash(b"ab"), (0x61u32 << 6) ^ 0x62);
        assert_eq!(unsafe { filename_hash_ptr(std::ptr::null()) }, 0xFFFF_FFFF);
    }
}
