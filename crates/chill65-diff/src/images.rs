//! Building the ROM images the harness runs.
//!
//! The game source is not in this repository and never will be (plan §9), so
//! everything here is gated on `CHILL65_CORPUS` and callers skip cleanly when
//! it is unset — the convention `chill65-runtime/tests/common/mod.rs` already
//! follows.
//!
//! The images are **build artefacts containing game-derived bytes**. They are
//! written under `target/`, which is gitignored, and must never be committed.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `A000-FFFF`.
pub const PROG_LEN: usize = 0x6000;
/// `A000-DFFF`.
pub const DATA_LEN: usize = 0x4000;

/// The corpus root, if the environment names one.
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

/// The two images, plus the symbol table that names what is in the program.
pub struct Images {
    pub prog: Vec<u8>,
    pub data: Vec<u8>,
    /// Path to the program's `.sym` file — routine attribution reads this.
    pub sym: PathBuf,
}

fn assemble(
    corpus: &Path,
    out: &Path,
    roots: &[&str],
    includes: &[&str],
    base: &str,
    end: &str,
    symbols: Option<&Path>,
) -> Result<(), String> {
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
    if let Some(sym) = symbols {
        cmd.arg("--symbols").arg(sym);
    }

    let status = cmd.status().map_err(|e| format!("running chill65-asm: {e}"))?;
    if !status.success() {
        return Err(format!("assembler failed for {roots:?}"));
    }
    Ok(())
}

/// Assemble the program and castle-data images from the corpus.
///
/// Identical to what `chill65-runtime/tests/common/mod.rs` does, with the
/// symbol table requested as well: the harness needs it to name routines.
pub fn build_images(corpus: &Path) -> Result<Images, String> {
    let out_dir = workspace_root().join("target/boot-artefacts");
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("{}: {e}", out_dir.display()))?;

    let prog_path = out_dir.join("prog.bin");
    let data_path = out_dir.join("data.bin");
    let sym_path = out_dir.join("prog.sym");

    assemble(
        corpus,
        &prog_path,
        &["CRF.MAC", "CRP.MAC", "CLS.MAC"],
        &["version-3", "."],
        "A000",
        "FFFF",
        Some(&sym_path),
    )?;
    assemble(corpus, &data_path, &["C99.MAC"], &["."], "A000", "DFFF", None)?;

    let prog = std::fs::read(&prog_path).map_err(|e| format!("{}: {e}", prog_path.display()))?;
    let data = std::fs::read(&data_path).map_err(|e| format!("{}: {e}", data_path.display()))?;
    if prog.len() != PROG_LEN {
        return Err(format!("program image is {} bytes, want {PROG_LEN}", prog.len()));
    }
    if data.len() != DATA_LEN {
        return Err(format!("data image is {} bytes, want {DATA_LEN}", data.len()));
    }

    Ok(Images {
        prog,
        data,
        sym: sym_path,
    })
}
