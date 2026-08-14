//! Tempest's attract mode: long, evolving, and identical twice over.
//!
//! The counterpart of `attract_sd.rs`, and the same bar `gate2.md` records for
//! Space Duel — 1,200 frames, the picture keeps changing, and two independent
//! runs agree frame for frame.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/tempest \
//!   cargo test -p chill65-runtime --test attract_te -- --ignored --nocapture
//! ```
//!
//! # A vector machine wedges without crashing
//!
//! `gate2.md`'s warning, and it applies here with a Tempest-shaped twist. If
//! the display list stops being rebuilt, the generator keeps drawing the last
//! one and `ALHARD.MAC:170-175` keeps restarting it, so the strobe count, the
//! interrupt count and the watchdog all look perfectly healthy. An evolving
//! frame hash is the only real signal of life.
//!
//! On this board the palette is part of that signal. `ALDISP.MAC:1078-1100`
//! rewrites `COLPORT` every frame, so `TeMachine::frame_hash` folds the
//! sixteen colour-RAM bytes in beside the segments — a picture whose geometry
//! is unchanged but whose colours moved is a different picture, and Tempest
//! animates exactly that way.
//!
//! # Why determinism is the point
//!
//! Phase 3's differential harness stands on it. The game reads POKEY `RANDOM`,
//! so two runs agreeing hash for hash proves the LFSR is clocked from machine
//! cycles and that nothing host-driven has leaked into the model.

mod common;

use chill65_runtime::cpu::{Cpu, CpuError};
use chill65_runtime::te::{run_frame_te, TeMachine, IRQS_PER_FRAME};

use common::{build_te_program, corpus, TeBuild, TE_PROG_BASE, TE_PROG_LEN, TE_VEC_ROM_BASE,
             TE_VEC_ROM_LEN};

/// Well past the boot sequence and deep into attract mode.
const FRAMES: u32 = 1_200;

/// The frame by which drawing has certainly started — `boot_te.rs` measured
/// the first segments at frame 17. Steady-state invariants start after this.
const STEADY_FROM: u32 = 40;

struct Outcome {
    cycles: u64,
    hashes: Vec<u64>,
    go_strobes: u64,
    irqs_taken: u64,
    watchdog_strobes: u64,
    segments: Vec<usize>,
}

fn run_attract(program: &TeBuild, frames: u32) -> Result<Outcome, CpuError> {
    let mut m = TeMachine::new();
    m.load_roms(
        &program.bytes(TE_VEC_ROM_BASE, TE_VEC_ROM_LEN),
        &program.bytes(TE_PROG_BASE, TE_PROG_LEN),
    )
    .expect("load");
    m.input.self_test = false;

    let mut cpu = Cpu::new();
    cpu.reset(&mut m);

    let mut hashes = Vec::with_capacity(frames as usize);
    let mut segments = Vec::with_capacity(frames as usize);
    let mut irqs_taken = 0u64;

    for _ in 0..frames {
        let stats = run_frame_te(&mut cpu, &mut m)?;
        irqs_taken += stats.irqs_taken as u64;
        assert_eq!(stats.irqs_raised, IRQS_PER_FRAME);
        hashes.push(m.frame_hash());
        segments.push(m.segments.len());
        assert!(!m.watchdog_expired, "the watchdog expired");
        assert!(m.vg_fault.is_none(), "generator fault: {:?}", m.vg_fault);
    }

    Ok(Outcome {
        cycles: m.cycles,
        hashes,
        go_strobes: m.vg_go_strobes,
        irqs_taken,
        watchdog_strobes: m.watchdog_strobes,
        segments,
    })
}

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
fn attract_mode_runs_draws_and_is_deterministic() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let program = build_te_program(&c);

    let first = run_attract(&program, FRAMES).expect("first run");
    let second = run_attract(&program, FRAMES).expect("second run");

    let distinct = {
        let mut h = first.hashes.clone();
        h.sort_unstable();
        h.dedup();
        h.len()
    };
    let quiet = first.segments[STEADY_FROM as usize..]
        .iter()
        .filter(|n| **n == 0)
        .count();
    println!(
        "{FRAMES} frames, {} cycles\n  \
         IRQs taken        {}\n  \
         VGSTART strobes   {}\n  \
         watchdog strobes  {}\n  \
         distinct pictures {distinct}\n  \
         empty frames after {STEADY_FROM}: {quiet}\n  \
         segments min/max after {STEADY_FROM}: {}/{}",
        first.cycles,
        first.irqs_taken,
        first.go_strobes,
        first.watchdog_strobes,
        first.segments[STEADY_FROM as usize..].iter().min().unwrap(),
        first.segments[STEADY_FROM as usize..].iter().max().unwrap(),
    );

    // A handful of blank frames is normal and a long run of them is not.
    //
    // Measured: six blank frames in 1,200, at 309-310, 386 and 930-932 — one
    // to three at a time, and the two clusters about 620 frames apart, which
    // at 27.8 frames a second is a twenty-two second cycle. That is the
    // attract sequence changing screens: the game clears the display list and
    // rebuilds it, and for a frame or two there is nothing to draw. A wedged
    // machine would show hundreds in a row, so the run length is what is
    // asserted rather than the count.
    let longest_blank = first.segments[STEADY_FROM as usize..]
        .split(|n| *n != 0)
        .map(|run| run.len())
        .max()
        .unwrap_or(0);
    println!("  longest blank run  {longest_blank}");
    assert!(
        longest_blank <= 5,
        "{longest_blank} blank frames in a row — the list stopped being rebuilt"
    );

    assert!(
        distinct > 100,
        "only {distinct} distinct pictures in {FRAMES} frames — \
         a vector machine wedges without crashing, and this is the only signal"
    );

    // Determinism, the property Phase 3 will stand on.
    assert_eq!(first.cycles, second.cycles, "cycle counts differ");
    assert_eq!(first.go_strobes, second.go_strobes);
    assert_eq!(first.irqs_taken, second.irqs_taken);
    assert_eq!(
        first.hashes, second.hashes,
        "two runs from the same inputs drew different pictures"
    );
}
