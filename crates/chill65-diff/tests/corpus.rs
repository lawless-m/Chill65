//! Every committed trace plays, plays the same way twice, and does something.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-diff --test corpus -- --ignored --nocapture
//! ```
//!
//! The third assertion is the one that earns its keep. A trace that parsed
//! cleanly and played deterministically could still be decorative — every
//! input dropped on the floor, indistinguishable from sitting in attract mode.
//! Requiring each non-idle trace to diverge from the idle baseline is what
//! makes the corpus evidence rather than furniture.

use std::path::PathBuf;

use chill65_diff::images::{build_images, corpus, workspace_root};
use chill65_diff::trace::Trace;
use chill65_diff::OurRuntime;

/// The trace every other one is compared against.
const BASELINE: &str = "idle-attract.trace";

fn traces_dir() -> PathBuf {
    workspace_root().join("traces")
}

/// Every `traces/*.trace`, sorted, baseline first.
fn trace_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(traces_dir())
        .expect("traces/ exists")
        .map(|e| e.expect("read dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "trace"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "traces/ holds no .trace files");
    files
}

fn load(path: &PathBuf) -> Trace {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Trace::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn every_committed_trace_plays_deterministically_and_changes_something() {
    let Some(game) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let images = build_images(&game).expect("build images");
    let player = || OurRuntime::new(images.prog.clone(), images.data.clone(), Some(images.mob.clone()));

    let files = trace_files();
    assert!(
        files.len() >= 5,
        "the corpus is meant to cover at least five situations, found {}",
        files.len()
    );

    // The baseline first, so the others have something to be measured against.
    let baseline_path = traces_dir().join(BASELINE);
    let baseline = load(&baseline_path);
    let baseline_hashes = player()
        .play(&baseline, baseline.len() as u32)
        .expect("baseline plays")
        .hashes;

    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let trace = load(path);
        let frames = trace.len() as u32;
        assert!(frames > 0, "{name} covers no frames");

        let first = player()
            .play(&trace, frames)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(
            !first.watchdog_expired,
            "{name}: the watchdog expired — the game crashed and a real board \
             would have reset itself, which is otherwise a silent failure"
        );

        let second = player()
            .play(&trace, frames)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        if let Some(f) = first
            .hashes
            .iter()
            .zip(&second.hashes)
            .position(|(a, b)| a != b)
        {
            panic!("{name}: two plays of the same trace diverged at frame {f}");
        }
        assert_eq!(first.cycles, second.cycles, "{name}: cycle counts diverged");

        if name == BASELINE {
            eprintln!("{name:24} {frames:5} frames — baseline");
            continue;
        }

        // Not decorative: somewhere in the shared prefix it must part company
        // with doing nothing at all.
        let diverged = baseline_hashes
            .iter()
            .zip(&first.hashes)
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| {
                panic!(
                    "{name}: produced exactly the idle picture stream over all \
                     {} shared frames — its input reaches nothing",
                    baseline_hashes.len().min(first.hashes.len())
                )
            });
        eprintln!("{name:24} {frames:5} frames — differs from idle at frame {diverged}");
    }
}
