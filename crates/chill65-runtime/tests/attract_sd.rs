//! Space Duel attract mode: it runs, it keeps drawing, and it does the same
//! thing twice.
//!
//! The counterpart of `attract.rs`, which does the same for Crystal Castles.
//!
//! # Why determinism is the interesting assertion
//!
//! It looks like a formality and is not. Space Duel reads POKEY's `RANDOM`,
//! and the attract sequence is not a fixed animation — it plays itself. So an
//! identical hash sequence across two independent runs is evidence that the
//! LFSR is clocked from machine cycles and that nothing host-dependent has
//! leaked in. A non-deterministic interpreter would make a differential
//! harness against MAME impossible to write later.
//!
//! # Why longevity is the other one
//!
//! A vector machine can wedge without crashing. If the display list stops
//! being rebuilt, the generator keeps drawing the last one perfectly happily
//! and the watchdog keeps being fed by the interrupt handler — so "no crash"
//! proves nothing on its own. A frame hash that stops changing is the signal
//! that matters, which is why this test watches the picture evolve rather than
//! only that it exists.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/space-duel \
//!   cargo test -p chill65-runtime --test attract_sd -- --ignored --nocapture
//! ```

mod common;

use chill65_runtime::sd::{run_frame_sd, SdMachine};
use chill65_runtime::{Cpu, CpuError};

use common::{build_sd_program, build_sd_ship, corpus, SD_PROG_LEN};

/// Well past the power-on self-test, which `selftest_sd.rs` measured as
/// finishing around frame 98, and deep into attract mode proper.
const FRAMES: u32 = 1_200;

/// The frame by which drawing has certainly started — measured in `boot_sd.rs`
/// as frame 99. Steady-state invariants are only asserted after this.
const STEADY_FROM: u32 = 120;

const OPTN1: u8 = 0x00;
const OPTN2: u8 = 0x00;

struct Outcome {
    cycles: u64,
    hashes: Vec<u64>,
    go_strobes: u64,
    irqs_taken: u64,
    watchdog_strobes: u64,
    /// Segments drawn on each frame, for the longevity assertions.
    segments: Vec<usize>,
}

fn run_attract(ship: &[u8], image: &[u8], frames: u32) -> Result<Outcome, CpuError> {
    let mut m = SdMachine::new();
    m.load_roms(ship, image).expect("load");
    m.set_options(OPTN1, OPTN2);
    m.self_test = false;

    let mut cpu = Cpu::new();
    cpu.reset(&mut m);

    let mut hashes = Vec::with_capacity(frames as usize);
    let mut segments = Vec::with_capacity(frames as usize);
    let mut irqs_taken = 0u64;

    for frame in 0..frames {
        let stats = run_frame_sd(&mut cpu, &mut m)?;
        irqs_taken += stats.irqs_taken as u64;
        hashes.push(m.frame_hash());
        segments.push(m.segments.len());

        assert_eq!(stats.irqs_raised, 4, "frame {frame}");
        assert!(
            !m.watchdog_expired,
            "the watchdog expired on frame {frame}: the machine wedged"
        );
        assert!(
            m.vg_fault.is_none(),
            "vector generator fault on frame {frame}: {:?}",
            m.vg_fault
        );
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
#[ignore = "needs CHILL65_CORPUS"]
fn attract_mode_runs_draws_and_is_deterministic() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let ship = build_sd_ship(&c);
    let program = build_sd_program(&c);
    let image = program.bytes(0x0000, SD_PROG_LEN);

    let first = run_attract(&ship, &image, FRAMES).expect("first run");
    let second = run_attract(&ship, &image, FRAMES).expect("second run");

    let distinct = {
        let mut h = first.hashes.clone();
        h.sort_unstable();
        h.dedup();
        h.len()
    };
    let steady = &first.segments[STEADY_FROM as usize..];
    println!(
        "{FRAMES} frames, {} cycles\n  \
         IRQs taken       {}\n  \
         GO strobes       {}\n  \
         watchdog strobes {}\n  \
         distinct hashes  {distinct}\n  \
         segments         min {} max {} last {}",
        first.cycles,
        first.irqs_taken,
        first.go_strobes,
        first.watchdog_strobes,
        steady.iter().min().unwrap(),
        steady.iter().max().unwrap(),
        first.segments.last().unwrap(),
    );

    // Longevity: the loop keeps running, and keeps drawing.
    assert!(
        first.watchdog_strobes >= FRAMES as u64,
        "{} watchdog strobes over {FRAMES} frames; ASTRD2.MAC:728 does one \
         per SYNC",
        first.watchdog_strobes
    );
    let steady_frames = (FRAMES - STEADY_FROM) as u64;
    assert!(
        first.go_strobes >= steady_frames / 2,
        "{} GO strobes over {steady_frames} steady-state frames",
        first.go_strobes
    );
    assert!(
        steady.iter().all(|&n| n > 0),
        "the screen went blank during attract mode"
    );

    // The picture must keep evolving. A wedged machine draws the same frame
    // forever without ever crashing, so this is the assertion that catches it.
    assert!(
        distinct > FRAMES as usize / 10,
        "only {distinct} distinct pictures across {FRAMES} frames — the \
         machine may be looping on one"
    );
    let tail: std::collections::BTreeSet<u64> =
        first.hashes[FRAMES as usize - 200..].iter().copied().collect();
    assert!(
        tail.len() > 1,
        "the picture stopped changing over the last 200 frames"
    );

    // The assertion this test exists for.
    assert_eq!(first.cycles, second.cycles, "cycle counts diverged");
    assert_eq!(
        first.irqs_taken, second.irqs_taken,
        "interrupt counts diverged"
    );
    assert_eq!(
        first.go_strobes, second.go_strobes,
        "GO strobe counts diverged"
    );
    match first
        .hashes
        .iter()
        .zip(&second.hashes)
        .position(|(a, b)| a != b)
    {
        None => {}
        Some(frame) => panic!(
            "the two runs diverged at frame {frame}: {:016x} against {:016x}",
            first.hashes[frame], second.hashes[frame]
        ),
    }
}
