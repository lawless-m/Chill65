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
