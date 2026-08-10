//! The verilated MiSTer core builds, runs, and draws.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-diff --test mister -- --ignored --nocapture
//! ```
//!
//! Skips cleanly unless verilator is on `PATH` and the corpus is set. Building
//! and running the simulation is slow — roughly half a second of wall clock per
//! emulated frame — so this is deliberately a modest number of frames.

use chill65_diff::images::corpus;
use chill65_diff::mister;
use chill65_diff::reference::FRAME_BYTES;

/// Past the point the game first draws, which `boot.rs` measures at frame 162.
const FRAMES: u32 = 240;

#[test]
#[ignore = "needs verilator on PATH and CHILL65_CORPUS"]
fn the_verilated_core_runs_and_draws() {
    if !mister::have_verilator() {
        eprintln!("verilator not on PATH — skipping");
        return;
    }
    let Some(game) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let capture = mister::capture(&game, FRAMES).expect("run the simulation");
    assert_eq!(capture.frames.len(), FRAMES as usize);
    for (k, f) in capture.frames.iter().enumerate() {
        assert_eq!(f.len(), FRAME_BYTES, "frame {k} is the wrong size");
    }

    // The core cannot emit the whole line: HBLANK2 blanks RGBout over the first
    // columns, so 252 of 256 is the honest figure and a change here means the
    // sync chain moved.
    eprintln!("emitted {} pixels per line", capture.emitted);
    assert_eq!(
        capture.emitted, 252,
        "the observable window changed; comparisons must be re-scoped"
    );

    let lit: Vec<usize> = capture
        .frames
        .iter()
        .map(|f| f.chunks_exact(3).filter(|p| p != &[0, 0, 0]).count())
        .collect();
    let best = lit.iter().copied().max().expect("frames captured");
    eprintln!(
        "lit pixels: first {}, last {}, most {best}",
        lit[0],
        lit[FRAMES as usize - 1]
    );
    assert!(best > 0, "the core drew nothing at all");

    let distinct: std::collections::HashSet<&Vec<u8>> = capture.frames.iter().collect();
    eprintln!("{} distinct images across {FRAMES} frames", distinct.len());
    assert!(distinct.len() > 1, "every frame was identical");
}
