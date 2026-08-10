//! CRC-32, the IEEE variant — what both ZIP archives and MAME's ROM audit use.
//!
//! Hand-rolled, like everything else here: the workspace takes no third-party
//! crates. Table-driven from the reflected polynomial `0xEDB88320`.

/// The reflected IEEE 802.3 polynomial.
const POLY: u32 = 0xEDB8_8320;

fn table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

/// CRC-32 of `bytes`.
pub fn crc32(bytes: &[u8]) -> u32 {
    let table = table();
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc = (crc >> 8) ^ table[((crc ^ b as u32) & 0xFF) as usize];
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_published_vectors() {
        // The canonical CRC-32 check values.
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
        assert_eq!(crc32(b"abc"), 0x3524_41C2);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }

    #[test]
    fn a_single_changed_bit_changes_the_result() {
        assert_ne!(crc32(&[0x00; 32]), crc32(&[0x01; 32]));
        let mut bytes = vec![0xA5u8; 8192];
        let before = crc32(&bytes);
        bytes[4096] ^= 0x01;
        assert_ne!(before, crc32(&bytes));
    }
}
