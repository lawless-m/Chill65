//! Building Tempest's ROM images, as a library call.
//!
//! The counterpart of [`crate::sd`], and here for the same reason: Tempest's
//! link lives in `tests/gate_te.rs`, test code cannot be called from a binary
//! or another crate, and so anything outside `cargo test` that wants to run
//! Tempest — the runtime, a frontend — had no supported way to get the ROMs.
//!
//! # Copied, not shared
//!
//! The logic here is transcribed from that gate rather than shared with it, on
//! `sd.rs`'s reasoning: each then answers to its own needs. The gate's copy
//! exists to prove the bytes, this one exists to run the machine, and neither
//! can be bent out of shape by the other. `chill65-runtime`'s `roms_te` test
//! builds both ways and compares every byte, so a drift between them fails
//! loudly rather than quietly.
//!
//! # Two images, and the hole between them
//!
//! `ALEXEC.COM`'s `IMGFIL` step cuts the linked binary into twelve 2K parts:
//! `136002.011` and `.012` at `3000` and `3800`, then `.001` through `.010`
//! covering `9000` to `D800`. So the board takes a 4K vector ROM at `3000` and
//! a 20K program ROM at `9000`, and the `4000-8FFF` gap between them is I/O —
//! not a hole in any image, but a region no image describes.
//!
//! Unlike Space Duel, whose fifteen-module link still reports six errors while
//! producing an exact image, Tempest's twelve-module link assembles **clean**.

use std::path::Path;

use crate::assemble::{Assembler, DirProvider};

/// Where the relocatable sections begin, from `ALEXEC.MAP`'s Section Summary:
/// `. ABS. 0000 A8B0 ABS,OVR`.
pub const SECTION_ORIGIN: u16 = 0xA8B0;

/// The vector ROM's span: CPU `3000-3FFF`, `ALEXEC.COM`'s parts `136002.011`
/// and `.012`.
pub const VEC_ROM_BASE: u16 = 0x3000;
pub const VEC_ROM_LEN: usize = 0x1000;

/// The program ROM's span: CPU `9000-DFFF`, `ALEXEC.COM`'s parts `136002.001`
/// through `.010`.
pub const PROG_BASE: u16 = 0x9000;
pub const PROG_LEN: usize = 0x5000;

/// How many assembly errors the twelve-module link reports.
///
/// None. This is a **pin, not a tolerance**: an error means the link is no
/// longer the one `gate_te.rs` proved byte-identical, and [`images`] fails
/// rather than quietly handing back something else.
pub const EXPECTED_ERRORS: usize = 0;

/// The twelve link units, in `ALEXEC.COM`'s order.
///
/// **The order is load-bearing** — it decides where each section contribution
/// lands, so a permutation assembles to a different image.
pub const ROOTS: [&str; 12] = [
    "ALWELG.MAC",
    "ALSCOR.MAC",
    "ALDISP.MAC",
    "ALEXEC.MAC",
    "ALSOUN.MAC",
    "ALVROM.MAC",
    "ALCOIN.MAC",
    "ALLANG.MAC",
    "ALHARD.MAC",
    "ALTEST.MAC",
    "ALEARO.MAC",
    "ALVGUT.MAC",
];

/// Tempest's two ROM images, ready for the runtime's `load_roms`.
pub struct TeImages {
    /// CPU `3000-3FFF`, [`VEC_ROM_LEN`] bytes.
    pub vec_rom: Vec<u8>,
    /// CPU `9000-DFFF`, [`PROG_LEN`] bytes.
    pub program: Vec<u8>,
}

/// Assemble both images from a directory of Tempest source.
///
/// `corpus` is the directory holding the `.MAC` files. Nothing is written to
/// disk and nothing is cached: the images are game-derived and live only in
/// memory, as everywhere else in this project.
///
/// One link produces both — the vector ROM and the program are two windows on
/// a single linked binary, which is exactly what `IMGFIL` cut them from.
pub fn images(corpus: &Path) -> Result<TeImages, String> {
    let image = build(corpus, &ROOTS, EXPECTED_ERRORS)?;
    Ok(TeImages {
        vec_rom: span(&image, VEC_ROM_BASE, VEC_ROM_LEN),
        program: span(&image, PROG_BASE, PROG_LEN),
    })
}

/// Assemble one link, keeping the image on the error path.
///
/// This is a library, so a deviation is an `Err` rather than a panic — a caller
/// with the wrong corpus deserves a message, not a crash.
fn build(
    corpus: &Path,
    roots: &[&str],
    expected: usize,
) -> Result<std::collections::BTreeMap<u16, u8>, String> {
    let provider = DirProvider {
        root: corpus.to_path_buf(),
    };
    let mut a = Assembler::new(&provider);
    // Tempest's relocatable region does not start where Space Duel's does.
    a.section_origin = SECTION_ORIGIN;
    let (image, errors) = match a.assemble_units(roots) {
        Ok(img) => (img, Vec::new()),
        Err(errs) => (std::mem::take(&mut a.image), errs),
    };

    if errors.len() != expected {
        return Err(format!(
            "{}: expected exactly {expected} assembly errors, got {}: {errors:#?}",
            roots.first().copied().unwrap_or("(no roots)"),
            errors.len()
        ));
    }
    if image.is_empty() {
        return Err(format!(
            "{}: the assembler produced no bytes — is {} a Tempest source directory?",
            roots.first().copied().unwrap_or("(no roots)"),
            corpus.display()
        ));
    }

    Ok(image)
}

/// Flatten a span of the address-keyed image, zero-filling anything unwritten.
fn span(image: &std::collections::BTreeMap<u16, u8>, base: u16, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| {
            image
                .get(&base.wrapping_add(i as u16))
                .copied()
                .unwrap_or(0)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The link order is a fact about `ALEXEC.COM`, and a permutation would
    /// assemble to a different image, so it is worth asserting it has not been
    /// tidied into alphabetical order by a helpful hand.
    #[test]
    fn the_link_order_is_the_one_alexec_com_uses() {
        assert_eq!(ROOTS.len(), 12);
        assert_eq!(ROOTS[0], "ALWELG.MAC", "ALWELG leads the link");
        assert_eq!(ROOTS[11], "ALVGUT.MAC", "ALVGUT closes it");
        let mut sorted = ROOTS;
        sorted.sort_unstable();
        assert_ne!(ROOTS, sorted, "the order is ALEXEC.COM's, not alphabetical");
    }

    /// The spans are `ALEXEC.COM`'s twelve 2K parts, in two runs.
    #[test]
    fn the_spans_are_the_parts_imgfil_cuts() {
        assert_eq!(VEC_ROM_LEN, 0x800 * 2, "two 2K parts at 3000 and 3800");
        assert_eq!(PROG_LEN, 0x800 * 10, "ten 2K parts from 9000 to D800");
        assert_eq!(
            VEC_ROM_BASE as usize + VEC_ROM_LEN,
            0x4000,
            "the vector ROM ends where the I/O region begins"
        );
        assert_eq!(
            PROG_BASE as usize + PROG_LEN,
            0xE000,
            "the program ROM ends at the top of the address space"
        );
    }

    /// No corpus needed: a directory with no source in it must be reported,
    /// not panicked over. Deliberately matched rather than `unwrap_err`, which
    /// would want `Debug` on [`TeImages`] and so put 24K of ROM in a panic
    /// message.
    #[test]
    fn a_directory_without_the_source_is_an_error_not_a_panic() {
        match images(Path::new("/nonexistent/tempest")) {
            Ok(_) => panic!("a missing corpus somehow assembled"),
            Err(e) => assert!(!e.is_empty(), "the error says something"),
        }
    }
}
