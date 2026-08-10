//! A minimal ZIP writer — enough for MAME to read a ROM set, no more.
//!
//! Only the `stored` method, so there is no compression to implement: a local
//! header, the bytes, and a central directory. The one non-obvious requirement
//! is the CRC-32 of each member, which a ROM set has to compute anyway.
//!
//! Timestamps are fixed rather than taken from the clock, so building the same
//! set twice gives byte-identical archives. There is no value in an archive
//! that changes every time it is rebuilt, and a great deal of value in one that
//! does not.

use crate::crc32::crc32;

/// 1 January 1983, in MS-DOS date format — the game's year, and a constant.
const DOS_DATE: u16 = (3 << 9) | (1 << 5) | 1;
const DOS_TIME: u16 = 0;

const LOCAL_SIG: u32 = 0x0403_4B50;
const CENTRAL_SIG: u32 = 0x0201_4B50;
const END_SIG: u32 = 0x0605_4B50;
/// PKZIP 2.0, which is what `stored` entries declare.
const VERSION: u16 = 20;

struct Entry {
    name: String,
    crc: u32,
    len: u32,
    offset: u32,
}

/// Builds a ZIP archive in memory.
#[derive(Default)]
pub struct ZipWriter {
    out: Vec<u8>,
    entries: Vec<Entry>,
}

impl ZipWriter {
    pub fn new() -> Self {
        ZipWriter::default()
    }

    /// Add one stored member.
    pub fn add(&mut self, name: &str, bytes: &[u8]) {
        let crc = crc32(bytes);
        let offset = self.out.len() as u32;

        self.out.extend_from_slice(&LOCAL_SIG.to_le_bytes());
        self.out.extend_from_slice(&VERSION.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes()); // flags
        self.out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        self.out.extend_from_slice(&DOS_TIME.to_le_bytes());
        self.out.extend_from_slice(&DOS_DATE.to_le_bytes());
        self.out.extend_from_slice(&crc.to_le_bytes());
        self.out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        self.out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes()); // extra field
        self.out.extend_from_slice(name.as_bytes());
        self.out.extend_from_slice(bytes);

        self.entries.push(Entry {
            name: name.to_string(),
            crc,
            len: bytes.len() as u32,
            offset,
        });
    }

    /// Finish the archive and hand back its bytes.
    pub fn finish(mut self) -> Vec<u8> {
        let cd_offset = self.out.len() as u32;
        for e in &self.entries {
            self.out.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
            self.out.extend_from_slice(&VERSION.to_le_bytes()); // made by
            self.out.extend_from_slice(&VERSION.to_le_bytes()); // needed
            self.out.extend_from_slice(&0u16.to_le_bytes()); // flags
            self.out.extend_from_slice(&0u16.to_le_bytes()); // stored
            self.out.extend_from_slice(&DOS_TIME.to_le_bytes());
            self.out.extend_from_slice(&DOS_DATE.to_le_bytes());
            self.out.extend_from_slice(&e.crc.to_le_bytes());
            self.out.extend_from_slice(&e.len.to_le_bytes());
            self.out.extend_from_slice(&e.len.to_le_bytes());
            self.out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            self.out.extend_from_slice(&0u16.to_le_bytes()); // extra
            self.out.extend_from_slice(&0u16.to_le_bytes()); // comment
            self.out.extend_from_slice(&0u16.to_le_bytes()); // disk
            self.out.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
            self.out.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            self.out.extend_from_slice(&e.offset.to_le_bytes());
            self.out.extend_from_slice(e.name.as_bytes());
        }
        let cd_size = self.out.len() as u32 - cd_offset;
        let count = self.entries.len() as u16;

        self.out.extend_from_slice(&END_SIG.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes()); // this disk
        self.out.extend_from_slice(&0u16.to_le_bytes()); // disk with cd
        self.out.extend_from_slice(&count.to_le_bytes());
        self.out.extend_from_slice(&count.to_le_bytes());
        self.out.extend_from_slice(&cd_size.to_le_bytes());
        self.out.extend_from_slice(&cd_offset.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes()); // comment
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut z = ZipWriter::new();
        z.add("first.bin", b"hello");
        z.add("second.bin", &[0xA5u8; 300]);
        z.finish()
    }

    #[test]
    fn has_the_expected_signatures_and_member_count() {
        let zip = sample();
        assert_eq!(&zip[0..4], &LOCAL_SIG.to_le_bytes(), "local header first");
        let end = zip.len() - 22;
        assert_eq!(&zip[end..end + 4], &END_SIG.to_le_bytes());
        // Members recorded twice: 2 in this disk, 2 in total.
        assert_eq!(u16::from_le_bytes([zip[end + 8], zip[end + 9]]), 2);
        assert_eq!(u16::from_le_bytes([zip[end + 10], zip[end + 11]]), 2);
    }

    #[test]
    fn stores_member_bytes_verbatim() {
        let zip = sample();
        // Stored, not compressed: the payload is findable as-is.
        assert!(
            zip.windows(5).any(|w| w == b"hello"),
            "the member's bytes should appear unaltered"
        );
    }

    #[test]
    fn records_each_members_crc() {
        let zip = sample();
        let want = crc32(b"hello").to_le_bytes();
        assert!(zip.windows(4).any(|w| w == want), "member CRC is written");
    }

    #[test]
    fn building_twice_gives_identical_bytes() {
        // Nothing may come from the clock; a set that changes each rebuild is
        // useless as a checkable artefact.
        assert_eq!(sample(), sample());
    }
}
