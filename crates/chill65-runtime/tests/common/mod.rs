//! Shared corpus handling for the gated integration tests.
//!
//! The game source is not in this repository and never will be (plan §9), so
//! every test that needs it is `#[ignore]`d and gated on `CHILL65_CORPUS`.
//! The images built here are **build artefacts containing game-derived bytes**;
//! they are written under `target/`, which is gitignored, and must never be
//! committed.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub const PROG_LEN: usize = 0x6000; // A000-FFFF
pub const DATA_LEN: usize = 0x4000; // A000-DFFF
pub const MOB_LEN: usize = 0x4000; // 136022-106.8d + 136022-107.8b

pub fn corpus() -> Option<PathBuf> {
    std::env::var("CHILL65_CORPUS").ok().map(PathBuf::from)
}

/// The workspace root, derived from this crate's manifest directory.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// Assemble one image with the Phase 1 assembler, through its real CLI.
fn assemble(corpus: &Path, out: &Path, roots: &[&str], includes: &[&str], base: &str, end: &str) {
    let root = workspace_root();
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&root)
        .args(["run", "-q", "-p", "chill65-asm", "--"])
        .args(roots);
    for inc in includes {
        cmd.arg("-I").arg(corpus.join(inc));
    }
    cmd.arg("-o")
        .arg(out)
        .args(["--base", base])
        .args(["--end", end]);

    let status = cmd.status().expect("failed to run chill65-asm");
    assert!(status.success(), "assembler failed for {roots:?}");
}

/// Build the program and castle-data images. Returns `(program, data)`.
pub fn build_images(corpus: &Path) -> (Vec<u8>, Vec<u8>) {
    let out_dir = workspace_root().join("target/boot-artefacts");
    std::fs::create_dir_all(&out_dir).expect("create artefact dir");

    let prog_path = out_dir.join("prog.bin");
    let data_path = out_dir.join("data.bin");

    assemble(
        corpus,
        &prog_path,
        &["CRF.MAC", "CRP.MAC", "CLS.MAC"],
        &["version-3", "."],
        "A000",
        "FFFF",
    );
    assemble(corpus, &data_path, &["C99.MAC"], &["."], "A000", "DFFF");

    let prog = std::fs::read(&prog_path).expect("read program image");
    let data = std::fs::read(&data_path).expect("read data image");
    assert_eq!(prog.len(), PROG_LEN, "program image size");
    assert_eq!(data.len(), DATA_LEN, "castle data image size");
    (prog, data)
}

/// The motion-object picture ROMs, as one 16384-byte image.
///
/// `372BR.RS4` in the corpus is exactly the two devices end to end —
/// `136022-106.8d` then `136022-107.8b`, 8192 bytes each — which is the same
/// slicing `chill65-diff`'s `romset.rs` uses to rebuild MAME's set, and the
/// order [`chill65_runtime::Machine::load_motion_roms`] expects.
///
/// Game-derived, like every other image here: it may be written under
/// gitignored `target/` and must never be committed (plan §9).
pub fn motion_rom_image(corpus: &Path) -> Vec<u8> {
    let path = corpus.join("372BR.RS4");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert!(
        bytes.len() >= MOB_LEN,
        "{} is {} bytes, need at least {MOB_LEN}",
        path.display(),
        bytes.len()
    );
    bytes[..MOB_LEN].to_vec()
}

// ---------------------------------------------------------------------------
// Space Duel
// ---------------------------------------------------------------------------

use std::collections::BTreeMap;

use chill65_asm::assemble::{Assembler, SourceProvider};

/// The ship image: `A2SHIP.LDA`'s span, CPU `2800-2FFF`.
pub const SD_SHIP_BASE: u16 = 0x2800;
pub const SD_SHIP_LEN: usize = 0x0800;

/// The program image: `AST2RD.LDA`'s span, CPU `0000-8FFF`.
pub const SD_PROG_LEN: usize = 0x9000;

/// The fifteen link units, in `SDGEN1.COM`'s order. Order is load-bearing:
/// it decides where each `.CSECT` contribution lands.
pub const SD_ROOTS: [&str; 15] = [
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

/// The assembly errors the fifteen-module link still reports.
///
/// **None, as of the HLL65 dialect work.** It reported six for most of this
/// project's life, all `expression ended unexpectedly`, and `gate1.md` §0a
/// records that the image was *exact* regardless — none of them ever changed
/// an emitted byte. Teaching the front end Tempest's five HLL65 rules
/// (`asm: five rules HLL65 needs and HLL65F never asked for`) resolved them
/// as a side effect, and the byte gate confirms the image did not move: still
/// 2,048 and 36,864 bytes exact.
///
/// This is a pin, not a tolerance. If the count or the kind ever moves, the
/// builders below fail loudly rather than quietly assembling something else.
pub const SD_EXPECTED_ERRORS: usize = 0;
pub const SD_EXPECTED_ERROR_TEXT: &str = "expression ended unexpectedly";

struct SdSearch {
    dir: PathBuf,
}

impl SourceProvider for SdSearch {
    fn load(&self, name: &str) -> Option<Vec<u8>> {
        for cand in [name.to_string(), format!("{name}.MAC")] {
            if let Ok(b) = std::fs::read(self.dir.join(&cand)) {
                return Some(b);
            }
        }
        None
    }
}

/// A Space Duel build: the image and every symbol the link resolved.
pub struct SdBuild {
    /// Address-keyed, exactly as the assembler produced it.
    pub image: BTreeMap<u16, u8>,
    /// `(address, name, unit)`, sorted — `"global"` for exported symbols.
    pub symbols: Vec<(u16, String, String)>,
}

impl SdBuild {
    /// Flatten a span into bytes, zero-filling anything unwritten.
    pub fn bytes(&self, base: u16, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| {
                self.image
                    .get(&base.wrapping_add(i as u16))
                    .copied()
                    .unwrap_or(0)
            })
            .collect()
    }

    /// Look a symbol up by name, six-character truncated as MACRO-11 does.
    pub fn symbol(&self, name: &str) -> Option<u16> {
        let key: String = name.chars().take(6).collect();
        self.symbols
            .iter()
            .find(|(_, n, _)| *n == key)
            .map(|(a, _, _)| *a)
    }
}

/// Assemble Space Duel, keeping the image on the error path.
///
/// This deliberately uses the assembler as a **library**. The CLI exits 1 on
/// any error, and the fifteen-module link still reports six benign ones, so
/// the `Command`-driven path the Crystal Castles builders use cannot work
/// here. `Assembler` leaves `image` populated when `assemble_units` returns
/// `Err`, which is what makes that recoverable — the same shape
/// `chill65-asm/tests/gate_sd.rs` uses, copied rather than shared so neither
/// can be bent out of shape by the other's needs.
fn build_sd(corpus: &Path, roots: &[&str], expected_errors: usize) -> SdBuild {
    let p = SdSearch {
        dir: corpus.to_path_buf(),
    };
    let mut a = Assembler::new(&p);
    let (image, errors) = match a.assemble_units(roots) {
        Ok(img) => (img, Vec::new()),
        Err(errs) => (std::mem::take(&mut a.image), errs),
    };

    assert_eq!(
        errors.len(),
        expected_errors,
        "expected exactly {expected_errors} assembly errors for {roots:?}, got {}: {errors:#?}",
        errors.len()
    );
    for e in &errors {
        assert!(
            e.contains(SD_EXPECTED_ERROR_TEXT),
            "unexpected assembly error kind: {e}"
        );
    }

    let mut symbols: Vec<(u16, String, String)> = a
        .globals
        .iter()
        .map(|(k, v)| (*v, k.clone(), "global".to_string()))
        .collect();
    for (unit, table) in &a.unit_locals {
        for (k, v) in table {
            if !k.contains('~') {
                symbols.push((*v, k.clone(), unit.clone()));
            }
        }
    }
    symbols.sort();
    SdBuild { image, symbols }
}

/// The ship build, image and symbols. `A2SHIP.MAC` is a clean single-module
/// assembly — no linker involved — so any error at all is a failure.
pub fn build_sd_ship_full(corpus: &Path) -> SdBuild {
    build_sd(corpus, &["A2SHIP.MAC"], 0)
}

/// The ship image alone, CPU `2800-2FFF`.
pub fn build_sd_ship(corpus: &Path) -> Vec<u8> {
    let bytes = build_sd_ship_full(corpus).bytes(SD_SHIP_BASE, SD_SHIP_LEN);
    assert_eq!(bytes.len(), SD_SHIP_LEN, "ship image size");
    bytes
}

/// The fifteen-module program link, CPU `0000-8FFF`, with its symbol table.
pub fn build_sd_program(corpus: &Path) -> SdBuild {
    build_sd(corpus, &SD_ROOTS, SD_EXPECTED_ERRORS)
}

// ---------------------------------------------------------------------------
// Tempest
// ---------------------------------------------------------------------------

/// The vector ROM: CPU `3000-3FFF`, `ALEXEC.COM`'s parts `136002.011`/`.012`.
pub const TE_VEC_ROM_BASE: u16 = 0x3000;
pub const TE_VEC_ROM_LEN: usize = 0x1000;

/// The program ROM: CPU `9000-DFFF`, parts `136002.001` through `.010`.
pub const TE_PROG_BASE: u16 = 0x9000;
pub const TE_PROG_LEN: usize = 0x5000;

/// Where Tempest's relocatable region begins. `ALEXEC.MAP`'s Section Summary
/// opens `. ABS. 0000 A8B0`, and `chill65-asm/tests/gate_te.rs` asserts all
/// eleven sections land on that table.
pub const TE_SECTION_ORIGIN: u16 = 0xA8B0;

/// The twelve link units, in `ALEXEC.COM`'s order. Order is load-bearing.
pub const TE_ROOTS: [&str; 12] = [
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

/// The assembly errors the twelve-module link reports: none.
///
/// A pin, not a tolerance. `gate_te.rs` proves this link byte-identical to
/// `TEMPST.LDA`, and it assembles clean doing it, so any error means the image
/// is no longer that one.
pub const TE_EXPECTED_ERRORS: usize = 0;

pub struct TeBuild {
    /// Address-keyed, exactly as the assembler produced it.
    pub image: BTreeMap<u16, u8>,
    /// `(address, name, unit)`, sorted — `"global"` for exported symbols.
    pub symbols: Vec<(u16, String, String)>,
}

impl TeBuild {
    /// Flatten a span into bytes, zero-filling anything unwritten.
    pub fn bytes(&self, base: u16, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| {
                self.image
                    .get(&base.wrapping_add(i as u16))
                    .copied()
                    .unwrap_or(0)
            })
            .collect()
    }

    /// Look a symbol up by name, six-character truncated as MACRO-11 does.
    pub fn symbol(&self, name: &str) -> Option<u16> {
        let key: String = name.chars().take(6).collect();
        self.symbols
            .iter()
            .find(|(_, n, _)| *n == key)
            .map(|(a, _, _)| *a)
    }
}

/// Assemble Tempest's twelve-module link, with its symbol table.
///
/// Copied from [`build_sd`] rather than shared, on the rule the gates follow:
/// each title's builder answers to its own needs and neither can be bent out
/// of shape by the other. The differences are the section origin and that this
/// link tolerates no errors at all.
pub fn build_te_program(corpus: &Path) -> TeBuild {
    let p = SdSearch {
        dir: corpus.to_path_buf(),
    };
    let mut a = Assembler::new(&p);
    a.section_origin = TE_SECTION_ORIGIN;
    let (image, errors) = match a.assemble_units(&TE_ROOTS) {
        Ok(img) => (img, Vec::new()),
        Err(errs) => (std::mem::take(&mut a.image), errs),
    };

    assert_eq!(
        errors.len(),
        TE_EXPECTED_ERRORS,
        "the twelve-module link must assemble clean, got {}: {errors:#?}",
        errors.len()
    );

    let mut symbols: Vec<(u16, String, String)> = a
        .globals
        .iter()
        .map(|(k, v)| (*v, k.clone(), "global".to_string()))
        .collect();
    for (unit, table) in &a.unit_locals {
        for (k, v) in table {
            if !k.contains('~') {
                symbols.push((*v, k.clone(), unit.clone()));
            }
        }
    }
    symbols.sort();
    TeBuild { image, symbols }
}
