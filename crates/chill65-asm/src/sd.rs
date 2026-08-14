//! Building Space Duel's ROM images, as a library call.
//!
//! Crystal Castles can be built by running the assembler CLI: it exits 0 and
//! writes an image. Space Duel cannot. Its fifteen-module link still reports
//! **six** assembly errors, the CLI exits 1 on any error at all, and yet the
//! image the link produces is byte-exact against the original — `gate1.md` §0a
//! records that. So the only way to build it is through the `Assembler` type
//! directly, keeping the image that `assemble_units` leaves behind when it
//! returns `Err`.
//!
//! That has been true since Phase 1, but it lived only in the test tree
//! (`chill65-runtime/tests/common/mod.rs` and `chill65-asm/tests/gate_sd.rs`).
//! Test code cannot be called from a binary, so anything outside `cargo test`
//! that wants to run Space Duel — a frontend, for instance — had no supported
//! way to get the ROMs. This module is that way.
//!
//! # Copied, not shared
//!
//! The logic here is transcribed from those two test builders rather than
//! shared with them, and they are deliberately left alone. Each then answers to
//! its own needs: the gate's copy exists to prove the bytes, this one exists to
//! run the machine, and neither can be bent out of shape by the other. If they
//! ever disagree, `chill65-runtime/tests/roms_sd.rs` fails — it builds both
//! ways and compares every byte.

use std::path::Path;

use crate::assemble::{Assembler, DirProvider};

/// The ship image's span: CPU `2800-2FFF`, from the `A2SHIP.LDA` link.
pub const SHIP_BASE: u16 = 0x2800;
pub const SHIP_LEN: usize = 0x0800;

/// The program image's span: CPU `0000-8FFF`, from the `AST2RD.LDA` link.
pub const PROG_BASE: u16 = 0x0000;
pub const PROG_LEN: usize = 0x9000;

/// The fifteen link units, in `SDGEN1.COM`'s order.
///
/// **The order is load-bearing** — it decides where each `.CSECT`
/// contribution lands, so a permutation assembles to a different image.
pub const ROOTS: [&str; 15] = [
    "AS2ROM.MAC",
    "ASTRD2.MAC",
    "AST2RT.MAC",
    "AS2SAC.MAC",
    "AS2POK.MAC",
    "AS2COI.MAC",
    "A2NAME.MAC",
    "AS2MSG.MAC",
    "AS2FIL.MAC",
    "AS2TST.MAC",
    "A2IRQ.MAC",
    "A2EARO.MAC",
    "XYSIG.MAC",
    "VGUTR2.MAC",
    "A2GOOF.MAC",
];

/// How many assembly errors the fifteen-module link reports, and of what kind.
///
/// **None, as of the HLL65 dialect work.** It reported six for most of this
/// project's life, all `expression ended unexpectedly`, and `gate1.md` §0a
/// shows the image was exact regardless, so none of them ever changed an
/// emitted byte. Teaching the front end Tempest's five HLL65 rules resolved
/// them as a side effect; the byte gate confirms the image did not move.
///
/// This is a **pin, not a tolerance**. An error, or a different kind, means
/// the image is no longer the one the gate proved, and [`images`] fails rather
/// than quietly handing back something else.
pub const EXPECTED_ERRORS: usize = 0;
pub const EXPECTED_ERROR_TEXT: &str = "expression ended unexpectedly";

/// Space Duel's two ROM images, ready for `SdMachine::load_roms`.
pub struct SdImages {
    /// CPU `2800-2FFF`, [`SHIP_LEN`] bytes.
    pub ship: Vec<u8>,
    /// CPU `0000-8FFF`, [`PROG_LEN`] bytes.
    pub program: Vec<u8>,
}

/// Assemble both images from a directory of Space Duel source.
///
/// `corpus` is the directory holding the `.MAC` and `.DAT` files. Nothing is
/// written to disk and nothing is cached: the images are game-derived and live
/// only in memory, as everywhere else in this project.
pub fn images(corpus: &Path) -> Result<SdImages, String> {
    let ship = build(corpus, &["A2SHIP.MAC"], 0)?;
    let program = build(corpus, &ROOTS, EXPECTED_ERRORS)?;
    Ok(SdImages {
        ship: span(&ship, SHIP_BASE, SHIP_LEN),
        program: span(&program, PROG_BASE, PROG_LEN),
    })
}

/// Assemble one link, keeping the image on the error path.
///
/// `expected` pins the error count: `A2SHIP.MAC` is a clean single-module
/// assembly and tolerates none, the fifteen-module link expects exactly
/// [`EXPECTED_ERRORS`]. This is a library, so a deviation is an `Err` rather
/// than a panic — a caller with the wrong corpus deserves a message, not a
/// crash.
fn build(
    corpus: &Path,
    roots: &[&str],
    expected: usize,
) -> Result<std::collections::BTreeMap<u16, u8>, String> {
    let provider = DirProvider {
        root: corpus.to_path_buf(),
    };
    let mut a = Assembler::new(&provider);
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
    if let Some(odd) = errors.iter().find(|e| !e.contains(EXPECTED_ERROR_TEXT)) {
        return Err(format!("unexpected assembly error kind: {odd}"));
    }
    if image.is_empty() {
        return Err(format!(
            "{}: the assembler produced no bytes — is {} a Space Duel source directory?",
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

    /// The link order is a fact about `SDGEN1.COM`, and a permutation would
    /// assemble to a different image, so it is worth asserting it has not been
    /// tidied into alphabetical order by a helpful hand.
    #[test]
    fn the_link_order_is_the_one_sdgen1_uses() {
        assert_eq!(ROOTS.len(), 15);
        assert_eq!(ROOTS[0], "AS2ROM.MAC", "AS2ROM leads the link");
        assert_eq!(ROOTS[14], "A2GOOF.MAC", "A2GOOF closes it");
        let mut sorted = ROOTS;
        sorted.sort_unstable();
        assert_ne!(ROOTS, sorted, "the order is SDGEN1's, not alphabetical");
    }

    /// No corpus needed: a directory with no source in it must be reported,
    /// not panicked over. Deliberately matched rather than `unwrap_err`, which
    /// would want `Debug` on [`SdImages`] and so put 36K of ROM in a panic
    /// message.
    #[test]
    fn a_directory_without_the_source_is_an_error_not_a_panic() {
        match images(Path::new("/nonexistent/space-duel")) {
            Ok(_) => panic!("a missing corpus somehow assembled"),
            Err(e) => assert!(!e.is_empty(), "the error says something"),
        }
    }
}
