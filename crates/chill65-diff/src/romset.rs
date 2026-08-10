//! Reconstructing MAME's `ccastles3` ROM set from the corpus.
//!
//! Every one of the eleven devices can be rebuilt from material the source tree
//! already contains, and each comes out matching MAME's own CRC-32 exactly — so
//! MAME's audit passes with no checksum bypass. Nothing is downloaded and
//! nothing game-derived enters this repository; the archive is written under
//! `target/`, which is gitignored.
//!
//! # Why `ccastles3` and not the parent set
//!
//! The parent `ccastles` is revision 4 (`136022-403/404/405`), and **no tree in
//! the corpus can build it**: the root gives 103/104/105, `version-2` gives
//! 203/204/205, `version-3` gives 303/304/305 (`integrity.md` §6 and §8).
//! Revision 3 is the one our assembler produces and therefore the one
//! `chill65-runtime` actually executes. Driving an external oracle on any other
//! revision would manufacture divergences that mean nothing.
//!
//! # The trap
//!
//! MAME locates a set by **zip filename**. The archive must be called
//! `ccastles3.zip` however tidy some other name might look.

use std::path::{Path, PathBuf};

use crate::crc32::crc32;
use crate::images::{build_images, workspace_root};
use crate::lda;
use crate::zip::ZipWriter;

/// Where a member's bytes come from.
enum Source {
    /// A slice of the assembled program image.
    Prog(usize, usize),
    /// A slice of the assembled castle-data image.
    Data(usize, usize),
    /// A slice of a raw file in the corpus root.
    Raw(&'static str, usize, usize),
    /// An `.LDA` PROM image, padded to 256 bytes.
    Prom(&'static str),
}

/// One device of the set: its MAME filename, where it comes from, and the
/// CRC-32 MAME expects.
struct Member {
    name: &'static str,
    source: Source,
    crc: u32,
}

/// The eleven devices of `ccastles3`.
///
/// The PROMs come from `version-2` deliberately: the root tree's four are
/// damaged — their record headers are malformed and they parse to nothing —
/// while version-2's match all four documented checksums (`integrity.md` §7).
const MEMBERS: [Member; 11] = [
    Member { name: "136022-303.1k", source: Source::Prog(0x0000, 0x2000), crc: 0x10E3_9FCE },
    Member { name: "136022-304.1l", source: Source::Prog(0x2000, 0x4000), crc: 0x7451_0F72 },
    Member { name: "136022-305.1n", source: Source::Prog(0x4000, 0x6000), crc: 0x9418_CF8A },
    Member { name: "136022-102.1h", source: Source::Data(0x0000, 0x2000), crc: 0xF6CC_FBD4 },
    Member { name: "136022-101.1f", source: Source::Data(0x2000, 0x4000), crc: 0xE2E1_7236 },
    Member { name: "136022-106.8d", source: Source::Raw("372BR.RS4", 0x0000, 0x2000), crc: 0x9D1D_89FC },
    Member { name: "136022-107.8b", source: Source::Raw("372BR.RS4", 0x2000, 0x4000), crc: 0x3996_0B7D },
    Member { name: "82s129-136022-108.7k", source: Source::Prom("version-2/TPSYNC.LDA"), crc: 0x6ED3_1E3B },
    Member { name: "82s129-136022-109.6l", source: Source::Prom("version-2/TPBUS.LDA"), crc: 0xB351_5F1A },
    Member { name: "82s129-136022-110.11l", source: Source::Prom("version-2/TOPOWP.LDA"), crc: 0x068B_DC7E },
    Member { name: "82s129-136022-111.10k", source: Source::Prom("version-2/TPI.LDA"), crc: 0xC29C_18D9 },
];

/// An 82S129 is 256 x 4; MAME's dumps are 256 bytes.
const PROM_LEN: usize = 0x100;

/// How one device came out.
pub struct Device {
    pub name: &'static str,
    pub len: usize,
    pub want: u32,
    pub got: u32,
}

impl Device {
    pub fn ok(&self) -> bool {
        self.want == self.got
    }
}

/// The rebuilt set.
pub struct RomSet {
    pub devices: Vec<Device>,
    pub zip: Vec<u8>,
}

impl RomSet {
    /// Every device matched the CRC MAME expects.
    pub fn all_ok(&self) -> bool {
        self.devices.iter().all(Device::ok)
    }
}

/// Where the archive belongs. The filename is not negotiable — see the module
/// documentation.
pub fn zip_path() -> PathBuf {
    workspace_root().join("target/boot-artefacts/roms/ccastles3.zip")
}

/// Rebuild the set from `corpus`, checking every device as it goes.
pub fn build(corpus: &Path) -> Result<RomSet, String> {
    let images = build_images(corpus)?;

    let mut devices = Vec::with_capacity(MEMBERS.len());
    let mut zip = ZipWriter::new();

    for member in &MEMBERS {
        let bytes: Vec<u8> = match &member.source {
            Source::Prog(from, to) => images.prog[*from..*to].to_vec(),
            Source::Data(from, to) => images.data[*from..*to].to_vec(),
            Source::Raw(file, from, to) => {
                let path = corpus.join(file);
                let blob =
                    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                if blob.len() < *to {
                    return Err(format!(
                        "{}: {} bytes, need {to}",
                        path.display(),
                        blob.len()
                    ));
                }
                blob[*from..*to].to_vec()
            }
            Source::Prom(file) => lda::load_padded(&corpus.join(file), PROM_LEN)?,
        };

        devices.push(Device {
            name: member.name,
            len: bytes.len(),
            want: member.crc,
            got: crc32(&bytes),
        });
        zip.add(member.name, &bytes);
    }

    Ok(RomSet {
        devices,
        zip: zip.finish(),
    })
}

/// Rebuild and write the archive, returning where it went.
pub fn build_and_write(corpus: &Path) -> Result<(RomSet, PathBuf), String> {
    let set = build(corpus)?;
    let path = zip_path();
    let dir = path.parent().expect("the archive has a parent directory");
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::write(&path, &set.zip).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((set, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_set_has_eleven_devices_with_distinct_names() {
        assert_eq!(MEMBERS.len(), 11);
        let mut names: Vec<&str> = MEMBERS.iter().map(|m| m.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate member name");
    }

    #[test]
    fn the_archive_is_named_for_the_set() {
        // MAME finds sets by zip filename; anything else silently fails to load.
        assert!(zip_path().ends_with("ccastles3.zip"));
    }

    #[test]
    fn every_slice_is_a_whole_device() {
        for m in &MEMBERS {
            let len = match m.source {
                Source::Prog(a, b) | Source::Data(a, b) | Source::Raw(_, a, b) => b - a,
                Source::Prom(_) => PROM_LEN,
            };
            assert!(
                len == 0x2000 || len == PROM_LEN,
                "{} is {len} bytes — neither a 2764 nor an 82S129",
                m.name
            );
        }
    }
}
