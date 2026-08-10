//! The rebuilt `ccastles3` set matches MAME's own CRC-32s, device for device.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-diff --test romset -- --ignored --nocapture
//! ```
//!
//! These eleven numbers are not ours. They are what MAME's ROM audit demands,
//! and matching every one of them is what lets the harness drive a real MAME on
//! a set built entirely from the game source, with no download and no checksum
//! bypass — and what makes it byte-for-byte the same code `chill65-runtime`
//! executes, which is the only reason a comparison between them means anything.

use chill65_diff::images::corpus;
use chill65_diff::romset::{build_and_write, zip_path};

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn every_device_matches_the_crc_mame_expects() {
    let Some(game) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let (set, path) = build_and_write(&game).expect("build the set");
    assert_eq!(set.devices.len(), 11, "the set is eleven devices");

    for d in &set.devices {
        eprintln!(
            "{} {:22} {:5} bytes  {:08X}",
            if d.ok() { "PASS" } else { "FAIL" },
            d.name,
            d.len,
            d.got
        );
    }
    let bad: Vec<String> = set
        .devices
        .iter()
        .filter(|d| !d.ok())
        .map(|d| format!("{}: got {:08X}, want {:08X}", d.name, d.got, d.want))
        .collect();
    assert!(bad.is_empty(), "devices do not match MAME:\n  {}", bad.join("\n  "));

    // Named for the set, because MAME finds sets by zip filename.
    assert_eq!(path, zip_path());
    assert!(path.ends_with("ccastles3.zip"));
    assert_eq!(
        std::fs::read(&path).expect("archive written").len(),
        set.zip.len()
    );

    // Under target/, never in the repository: these are game-derived bytes.
    assert!(
        path.components().any(|c| c.as_os_str() == "target"),
        "the archive escaped target/: {}",
        path.display()
    );

    // Deterministic, so the artefact can be checked rather than merely made.
    let (again, _) = build_and_write(&game).expect("rebuild");
    assert_eq!(set.zip, again.zip, "two builds produced different archives");
}
