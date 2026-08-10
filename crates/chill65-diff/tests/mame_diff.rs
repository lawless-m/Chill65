//! Driving MAME from a trace, and the first cross-implementation report.
//!
//! ```text
//! CHILL65_MAME=$(which mame) CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-diff --test mame_diff -- --ignored --nocapture
//! ```
//!
//! # This test does not assert the two agree
//!
//! It asserts the **mechanics**: both sides produce a full stream, a report is
//! generated, and the first divergent frame is the same on two independent
//! invocations. Whether our runtime and MAME draw the same picture is the
//! harness's *output*, not its pass condition — `gate2.md` lists six hardware
//! claims still marked UNVERIFIED and motion objects are not modelled at all,
//! so divergence is expected and informative. A test demanding equality would
//! be one the project cannot pass.

use std::path::PathBuf;

use chill65_diff::images::{build_images, corpus, workspace_root};
use chill65_diff::localise::localise_external;
use chill65_diff::mame::MameReference;
use chill65_diff::reference::Reference;
use chill65_diff::symbols::Symbols;
use chill65_diff::trace::Trace;
use chill65_diff::OurRuntime;

/// Long enough to be past the self-test and drawing.
const FRAMES: u32 = 600;
/// Enough of `coin-start.trace` to include the coin at frame 400.
const INPUT_FRAMES: u32 = 500;

fn trace(name: &str) -> Trace {
    let path = workspace_root().join("traces").join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Trace::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn report_path() -> PathBuf {
    workspace_root().join("target/boot-artefacts/diff/mame-attract.txt")
}

#[test]
#[ignore = "needs CHILL65_MAME and CHILL65_CORPUS"]
fn mame_is_deterministic_and_the_first_divergence_is_reproducible() {
    let Some(game) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let Some(mut mame) = MameReference::discover(game.clone()) else {
        eprintln!("CHILL65_MAME unset — skipping");
        return;
    };

    // (2) The adapter must be deterministic, or nothing built on it means
    // anything. Driven from a trace carrying real input, not just idle.
    let driven = trace("coin-start.trace");
    let first = mame.hashes(&driven, INPUT_FRAMES).expect("first MAME run");
    let second = mame.hashes(&driven, INPUT_FRAMES).expect("second MAME run");
    assert_eq!(first.len(), INPUT_FRAMES as usize);
    if let Some(f) = first.iter().zip(&second).position(|(a, b)| a != b) {
        panic!(
            "two identical MAME runs diverged at frame {f} — state is leaking \
             between runs (cfg, nvram, or something clock-driven)"
        );
    }
    eprintln!("mame determinism: {INPUT_FRAMES} frames identical across two runs");

    // Input reaches MAME at all: the coin-start trace must not produce the
    // same stream as sitting idle.
    let idle = mame
        .hashes(&Trace::idle(INPUT_FRAMES as usize), INPUT_FRAMES)
        .expect("idle MAME run");
    let differs = first.iter().zip(&idle).position(|(a, b)| a != b);
    match differs {
        Some(f) => eprintln!("mame input:       coin-start differs from idle at frame {f}"),
        None => eprintln!(
            "mame input:       WARNING — coin-start produced the idle stream; \
             either the injection is not landing or MAME's coin handling \
             differs from ours. Recorded, not asserted."
        ),
    }

    // (3) The first real differential report.
    let images = build_images(&game).expect("build images");
    let symbols = Symbols::load(&images.sym).expect("symbols");
    let attract = trace("idle-attract.trace");
    let mut ours = OurRuntime::new(images.prog.clone(), images.data.clone());

    let report = localise_external(&mut ours, &mut mame, &attract, FRAMES, &symbols)
        .expect("localise against MAME");
    eprintln!("ours vs mame:     {report}");

    assert_eq!(
        report.comparison.lengths, None,
        "the two sides produced different numbers of frames"
    );
    assert_eq!(report.comparison.compared, FRAMES as usize);

    let path = report_path();
    std::fs::create_dir_all(path.parent().unwrap()).expect("create diff dir");
    let mut text = format!(
        "chill65-diff: our runtime vs MAME\n\
         trace:  traces/idle-attract.trace\n\
         frames: {FRAMES}\n\
         set:    ccastles3, rebuilt from the corpus\n\n\
         {report}\n"
    );
    if !report.blame.is_empty() {
        text.push_str("\nblame, most pixels first:\n");
        for b in &report.blame {
            text.push_str(&format!("  {:8} {}\n", b.pixels, b.symbol));
        }
    }
    text.push_str(
        "\nDivergence here is expected output, not failure: six hardware claims\n\
         remain UNVERIFIED (gate2.md) and motion objects are not modelled.\n",
    );
    std::fs::write(&path, &text).expect("write report");
    eprintln!("wrote {}", path.display());

    // And the answer must be the same the second time round.
    let mut ours_again = OurRuntime::new(images.prog, images.data);
    let again = localise_external(&mut ours_again, &mut mame, &attract, FRAMES, &symbols)
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
