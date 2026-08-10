//! Our runtime against the verilated MiSTer core.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-diff --test mister_diff -- --ignored --nocapture
//! ```
//!
//! As with MAME, this asserts **mechanics and reproducibility, never
//! equality**. Divergence between our model and an FPGA core is the harness's
//! output; `gate2.md` lists six hardware claims still marked UNVERIFIED and
//! motion objects are modelled but not yet calibrated against either oracle.
//!
//! The simulation costs roughly half a second of wall clock per emulated frame,
//! so the determinism check runs short and the comparison runs once.

use std::path::PathBuf;

use chill65_diff::images::{build_images, corpus, workspace_root};
use chill65_diff::localise::localise_external;
use chill65_diff::mister::{self, MisterReference};
use chill65_diff::reference::Reference;
use chill65_diff::symbols::Symbols;
use chill65_diff::trace::Trace;
use chill65_diff::OurRuntime;

/// Far enough in for the core to be drawing: it reaches 176 lit pixels by
/// frame 350 and 32,573 by 450, against our runtime's first draw at 162.
const FRAMES: u32 = 500;
/// Short, because two runs at half a second a frame add up.
const DETERMINISM_FRAMES: u32 = 60;

fn report_path() -> PathBuf {
    workspace_root().join("target/boot-artefacts/diff/mister-attract.txt")
}

#[test]
#[ignore = "needs verilator on PATH and CHILL65_CORPUS"]
fn the_core_is_deterministic_and_the_divergence_is_reproducible() {
    if !mister::have_verilator() {
        eprintln!("verilator not on PATH — skipping");
        return;
    }
    let Some(game) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let mut core = MisterReference::discover(game.clone()).expect("verilator checked above");

    // Deterministic, with the cache cleared between runs so the second is a
    // genuine re-simulation rather than a replay.
    let short = Trace::idle(DETERMINISM_FRAMES as usize);
    let first = core.hashes(&short, DETERMINISM_FRAMES).expect("first run");
    core.forget();
    let second = core.hashes(&short, DETERMINISM_FRAMES).expect("second run");
    if let Some(f) = first.iter().zip(&second).position(|(a, b)| a != b) {
        panic!("two identical simulations diverged at frame {f}");
    }
    eprintln!("mister determinism: {DETERMINISM_FRAMES} frames identical across two runs");

    let images = build_images(&game).expect("build images");
    let symbols = Symbols::load(&images.sym).expect("symbols");
    let attract = {
        let path = workspace_root().join("traces/idle-attract.trace");
        Trace::parse(&std::fs::read_to_string(&path).expect("read trace")).expect("parse trace")
    };
    let mut ours = OurRuntime::new(images.prog.clone(), images.data.clone(), Some(images.mob.clone()));

    core.forget();
    let report = localise_external(&mut ours, &mut core, &attract, FRAMES, &symbols)
        .expect("localise against the core");
    eprintln!("ours vs mister:   {report}");

    assert_eq!(report.comparison.compared, FRAMES as usize);
    assert_eq!(
        report.comparison.lengths, None,
        "the two sides produced different numbers of frames"
    );

    let path = report_path();
    std::fs::create_dir_all(path.parent().unwrap()).expect("create diff dir");
    let mut text = format!(
        "chill65-diff: our runtime vs the verilated MiSTer core\n\
         trace:  traces/idle-attract.trace\n\
         frames: {FRAMES}\n\
         core:   Arcade-CrystalCastles_MiSTer, verilated unmodified\n\n\
         {report}\n"
    );
    if !report.blame.is_empty() {
        text.push_str("\nblame, most pixels first:\n");
        for b in &report.blame {
            text.push_str(&format!("  {:8} {}\n", b.pixels, b.symbol));
        }
    }
    text.push_str(
        "\nCaveats that bound this comparison:\n\
         - the core emits 252 of 256 columns; the rest cannot be observed at\n\
           its ports at all (HBLANK2 blanks RGBout).\n\
         - our colour RAM powers on zeroed, the core's has entry 16 set to 1FF,\n\
           so frame 0 differs for reasons unrelated to drawing.\n\
         - divergence here is expected output, not failure: six hardware claims\n\
           remain UNVERIFIED (gate2.md), and motion objects are modelled\n\
           from the RTL but not yet calibrated against an oracle.\n",
    );
    std::fs::write(&path, &text).expect("write report");
    eprintln!("wrote {}", path.display());

    // Reproducible: the cached run is reused for our side's re-derivation, so
    // this checks the harness rather than the simulator.
    let mut ours_again = OurRuntime::new(images.prog, images.data, Some(images.mob));
    let again = localise_external(&mut ours_again, &mut core, &attract, FRAMES, &symbols)
        .expect("second localise");
    assert_eq!(
        again.comparison.first_divergence, report.comparison.first_divergence,
        "the first divergent frame moved between invocations"
    );
    assert_eq!(
        again.blame.first().map(|b| &b.symbol),
        report.blame.first().map(|b| &b.symbol),
        "the top-blamed routine moved between invocations"
    );
}
