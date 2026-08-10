//! `verify` and `coverage` across the whole committed trace corpus.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-native -- --ignored --nocapture
//! ```
//!
//! Skips cleanly without the corpus.
//!
//! Two assertions. Every trace must be **identical** dispatch-off versus
//! dispatch-on — that is what makes a migration safe rather than merely fast.
//! And the aggregate compiled share must **exceed 50%**, which is Phase 4's
//! stated terminus made machine-checkable.
//!
//! This test was written when the manifest was empty and the share was 0%, so
//! the meter has been seen reading zero as well as reading a majority.

use std::collections::HashMap;
use std::path::PathBuf;

use chill65_diff::images::{build_images, corpus, workspace_root};
use chill65_diff::trace::Trace;
use chill65_diff::OurRuntime;
use chill65_native::Registry;
use chill65_runtime::frame::NoCompiled;

/// Every committed trace, sorted.
fn traces() -> Vec<PathBuf> {
    let dir = workspace_root().join("traces");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("traces/ exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "trace"))
        .collect();
    files.sort();
    files
}

fn load(path: &PathBuf) -> Trace {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Trace::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn every_trace_verifies_and_the_metric_reports() {
    let Some(game) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let images = build_images(&game).expect("build images");
    let files = traces();
    assert!(files.len() >= 5, "expected the five committed traces");

    let mut total = 0u64;
    let mut compiled_total = 0u64;
    let mut per_trace: HashMap<String, (u64, u64)> = HashMap::new();

    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let trace = load(path);
        let frames = trace.len() as u32;
        let mut ours = OurRuntime::new(images.prog.clone(), images.data.clone());

        let interpreted = ours
            .play_with(&trace, frames, &mut NoCompiled)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut registry = Registry::game();
        let dispatched = ours
            .play_with(&trace, frames, &mut registry)
            .unwrap_or_else(|e| panic!("{name}: {e}"));

        // The assertion every migrated routine will face.
        if let Some(f) = interpreted
            .hashes
            .iter()
            .zip(&dispatched.hashes)
            .position(|(a, b)| a != b)
        {
            panic!("{name}: dispatch changed the picture at frame {f}");
        }
        assert_eq!(
            interpreted.cycles, dispatched.cycles,
            "{name}: dispatch changed the cycle count"
        );

        let executed = dispatched.compiled + dispatched.interpreted;
        total += executed;
        compiled_total += dispatched.compiled;
        per_trace.insert(name.clone(), (dispatched.compiled, executed));

        eprintln!(
            "{name:24} {frames:5} frames  {executed:9} instructions  {:5.1}% compiled",
            if executed == 0 {
                0.0
            } else {
                100.0 * dispatched.compiled as f64 / executed as f64
            }
        );
    }

    let share = 100.0 * compiled_total as f64 / total as f64;
    eprintln!(
        "\naggregate: {compiled_total} of {total} executed instructions compiled ({share:.1}%)"
    );
    assert!(total > 0, "no instructions executed at all");

    // Whatever the manifest says must be reflected in the measurement: an
    // empty one compiles nothing, a populated one must actually be running.
    if Registry::game().is_empty() {
        assert_eq!(compiled_total, 0, "an empty manifest compiled something");
        return;
    }
    assert!(
        compiled_total > 0,
        "routines are in the manifest but none of them executed"
    );

    // Phase 4's terminus: a majority of the *running* code is compiled.
    // Executed instructions, so a hundred cold routines cannot buy it.
    assert!(
        compiled_total * 2 > total,
        "compiled share is {share:.1}%, which is not a majority — reopen \
         migration rather than weakening this"
    );
}
