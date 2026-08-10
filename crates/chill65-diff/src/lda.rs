//! The DEC absolute-loader `.LDA` record format.
//!
//! Records are `01 00 <count:16le> <addr:16le> <data…> <checksum>`, where
//! `count` spans the six-byte header and excludes the checksum byte. Later
//! records overwrite earlier ones.
//!
//! Prior art, and the source of that description: `load_lda` in
//! `chill65-asm/tests/gate.rs:46`, which uses it to read the original
//! toolchain's own output as the Phase 1 oracle. Here it reads the four PROM
//! images instead.

use std::collections::BTreeMap;
use std::path::Path;

/// Parse an absolute-loader image into an address-keyed map.
pub fn parse(blob: &[u8]) -> BTreeMap<u16, u8> {
    let mut mem = BTreeMap::new();
    let mut off = 0usize;
    while off + 6 <= blob.len() && blob[off] == 0x01 {
        let count = blob[off + 2] as usize | ((blob[off + 3] as usize) << 8);
        let addr = blob[off + 4] as u16 | ((blob[off + 5] as u16) << 8);
        if count < 6 || off + count >= blob.len() {
            break;
        }
        for (i, b) in blob[off + 6..off + count].iter().enumerate() {
            mem.insert(addr.wrapping_add(i as u16), *b);
        }
        off += count + 1;
    }
    mem
}

/// Read an `.LDA` file as a fixed-size image starting at address zero, with
/// anything the records do not cover left as zero.
///
/// The padding is not a convenience: `TPI.LDA` records only 128 bytes, and
/// MAME's dump of the same 82S129 is 256 bytes with the upper half zero. The
/// two agree exactly once the image is padded.
pub fn load_padded(path: &Path, len: usize) -> Result<Vec<u8>, String> {
    let blob = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mem = parse(&blob);
    if mem.is_empty() {
        return Err(format!("{}: no absolute-loader records", path.display()));
    }
    if let Some(&highest) = mem.keys().next_back() {
        if highest as usize >= len {
            return Err(format!(
                "{}: records reach {highest:04X}, past the {len}-byte image",
                path.display()
            ));
        }
    }
    Ok((0..len).map(|a| *mem.get(&(a as u16)).unwrap_or(&0)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One record: header, three data bytes at 0002, then a checksum byte.
    const ONE: &[u8] = &[0x01, 0x00, 0x09, 0x00, 0x02, 0x00, 0xAA, 0xBB, 0xCC, 0x00];

    #[test]
    fn reads_a_record_at_its_address() {
        let mem = parse(ONE);
        assert_eq!(mem.len(), 3);
        assert_eq!(mem[&0x0002], 0xAA);
        assert_eq!(mem[&0x0004], 0xCC);
        assert!(!mem.contains_key(&0x0000));
    }

    #[test]
    fn later_records_overwrite_earlier_ones() {
        let mut blob = ONE.to_vec();
        blob.extend_from_slice(&[0x01, 0x00, 0x07, 0x00, 0x02, 0x00, 0x11, 0x00]);
        let mem = parse(&blob);
        assert_eq!(mem[&0x0002], 0x11, "the second record wins");
        assert_eq!(mem[&0x0003], 0xBB, "and leaves the rest alone");
    }

    #[test]
    fn a_truncated_record_stops_the_parse_rather_than_panicking() {
        let mem = parse(&ONE[..7]);
        assert!(mem.is_empty());
    }

    #[test]
    fn non_record_data_yields_nothing() {
        assert!(parse(b"not an LDA file at all").is_empty());
    }
}
