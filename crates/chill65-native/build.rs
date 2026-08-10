//! Generate compiled routines at build time.
//!
//! Two independent generations, both into `OUT_DIR`:
//!
//! - **The fixture**, always. An original program committed in this crate, so
//!   `cargo build` and `cargo test` exercise the whole assemble → IR → emit →
//!   compile → run path with no game corpus present at all.
//! - **The game**, only when `CHILL65_CORPUS` is set, for the routine names in
//!   `routines.txt`.
//!
//! Emitted game Rust never leaves `OUT_DIR`. It is generated from Atari's
//! source and is exactly as undistributable as the ROM (plan §9); the manifest
//! holds routine *names*, which are our own reading of the source, and is
//! committable.

use std::path::{Path, PathBuf};

use chill65_asm::assemble::{Assembler, SourceProvider};
use chill65_asm::emit::emit;

/// Loads sources from a list of directories, `.MAC` implicit.
struct Dirs(Vec<PathBuf>);

impl SourceProvider for Dirs {
    fn load(&self, name: &str) -> Option<Vec<u8>> {
        for dir in &self.0 {
            for cand in [name.to_string(), format!("{name}.MAC")] {
                if let Ok(bytes) = std::fs::read(dir.join(&cand)) {
                    return Some(bytes);
                }
            }
        }
        None
    }
}

/// Flatten an assembled image into the 24576 bytes `Machine::load_roms` wants.
fn program_image(image: &std::collections::BTreeMap<u16, u8>) -> Vec<u8> {
    (0xA000..=0xFFFFu32)
        .map(|a| image.get(&(a as u16)).copied().unwrap_or(0))
        .collect()
}

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let crate_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));

    println!("cargo:rerun-if-env-changed=CHILL65_CORPUS");
    println!("cargo:rerun-if-changed=fixture/fixture.MAC");
    println!("cargo:rerun-if-changed=fixture/hll.MAC");
    println!("cargo:rerun-if-changed=routines.txt");

    generate_fixture(&crate_dir, &out);
    generate_game(&crate_dir, &out);
}

/// The always-present fixture: assemble, emit every routine, write the image.
fn generate_fixture(crate_dir: &Path, out: &Path) {
    let provider = Dirs(vec![crate_dir.join("fixture")]);
    let mut asm = Assembler::new(&provider);
    let image = match asm.assemble("fixture.MAC") {
        Ok(i) => i,
        Err(e) => panic!("fixture assembly failed:\n{e:#?}"),
    };

    std::fs::write(out.join("fixture.bin"), program_image(&image)).expect("write fixture image");

    let routines = asm.ir.routines();
    let names: Vec<&str> = routines.iter().map(String::as_str).collect();
    let emitted = emit(&asm.ir, &names);

    // A fixture routine that cannot be lowered is a bug in the fixture or the
    // emitter, not a fact about the game, so say so loudly at build time.
    for r in &emitted.refused {
        println!("cargo:warning=fixture routine {} refused: {}", r.routine, r.reason);
    }
    assert!(
        !emitted.functions.is_empty(),
        "the fixture lowered nothing; refusals: {:?}",
        emitted.refused
    );

    std::fs::write(out.join("fixture_compiled.rs"), &emitted.source).expect("write fixture code");
}

/// The game's routines, when a corpus is available. Empty otherwise, and
/// everything still builds.
fn generate_game(crate_dir: &Path, out: &Path) {
    let manifest = std::fs::read_to_string(crate_dir.join("routines.txt")).unwrap_or_default();
    let wanted: Vec<String> = manifest
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let Some(corpus) = std::env::var("CHILL65_CORPUS").ok().map(PathBuf::from) else {
        // No corpus: an empty registry, so the crate builds and its own tests
        // run anywhere.
        std::fs::write(out.join("game_compiled.rs"), empty_registry("no corpus"))
            .expect("write empty game registry");
        return;
    };
    if wanted.is_empty() {
        std::fs::write(out.join("game_compiled.rs"), empty_registry("empty manifest"))
            .expect("write empty game registry");
        return;
    }

    let provider = Dirs(vec![corpus.join("version-3"), corpus.clone()]);
    let mut asm = Assembler::new(&provider);
    let _image = match asm.assemble_units(&["CRF.MAC", "CRP.MAC", "CLS.MAC"]) {
        Ok(i) => i,
        Err(e) => panic!("game assembly failed:\n{e:#?}"),
    };

    let names: Vec<&str> = wanted.iter().map(String::as_str).collect();
    let emitted = emit(&asm.ir, &names);
    for r in &emitted.refused {
        println!("cargo:warning=routine {} refused: {}", r.routine, r.reason);
    }
    std::fs::write(out.join("game_compiled.rs"), &emitted.source).expect("write game code");
}

/// A registry with nothing in it, so callers need no `cfg`.
fn empty_registry(why: &str) -> String {
    format!(
        "// Generated: {why}.\n\
         use chill65_runtime::{{Cpu, Machine}};\n\n\
         pub const ROUTINES: &[(u16, fn(&mut Cpu, &mut Machine, u64) -> u64)] = &[];\n\
         pub const METADATA: &[(&str, u16, usize, &str)] = &[];\n"
    )
}
