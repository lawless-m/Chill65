//! Playing a trace on our own runtime: it is deterministic, and the input in
//! the trace actually reaches the machine.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-diff --test playback -- --ignored --nocapture
//! ```
//!
//! # Why these two assertions
//!
//! Determinism is the foundation the whole harness stands on: if a run of ours
//! is not repeatable, "diverged at frame N" is noise. `chill65-runtime`'s
//! `attract.rs` proved the machine deterministic; this proves the *trace
//! player* keeps it that way, which is a different claim — a player that leaked
//! host state or applied input at the wrong moment would break it.
//!
//! The second assertion is the one that stops the first being vacuous. A player
//! that dropped every input on the floor would be perfectly deterministic and
//! perfectly useless, so a trace carrying movement and coin/start presses must
//! produce a different picture from an idle one.

use chill65_diff::images::{build_images, corpus};
use chill65_diff::trace::{FrameInput, Switches, Trace};
use chill65_diff::{OurRuntime, Reference};
use chill65_runtime::Switch;

/// Well past the power-on self-test, which ends around frame 353
/// (`hardware.md` §10.2), and into attract mode proper.
const FRAMES: u32 = 600;

/// A trace that coins up, starts, and pushes the trackball about.
///
/// The timing is not arbitrary. Coin detection is Mike Albaugh's routine,
/// documented in the game's own source at `CCN.MAC:269-300`: the switch is
/// sampled **once per interrupt, four times a frame**, and a valid coin is
/// between 16 ms and 800 ms of contact flanked by 33 ms of no contact. So a
/// ten-frame press (167 ms) is comfortably valid — and a *longer* press is not
/// better, since past 800 ms (48 frames) the coin is rejected as too long.
///
/// The credit does not appear the moment the coin is accepted. `$PSTSL`, the
/// post-coin slam timer, then counts 120 down at four per frame — 30 frames —
/// before the credit is granted, so that a slam during the delay can invalidate
/// it. Pressing start before that has no effect at all, which is exactly the
/// mistake the first version of this test made.
fn busy_trace() -> Trace {
    /// Past the power-on self-test and settled into attract mode.
    const SETTLE: usize = 400;
    /// 167 ms of contact: valid, and well inside the 800 ms ceiling.
    const COIN_FRAMES: usize = 10;
    /// `$PSTSL` is 30 frames; leave margin for the 33 ms release either side.
    const CREDIT_DELAY: usize = 50;
    const START_FRAMES: usize = 10;

    let mut frames = vec![FrameInput::idle(); SETTLE];

    let coin: Switches = [Switch::CoinLeft].into_iter().collect();
    frames.extend(std::iter::repeat_n(
        FrameInput {
            switches: coin,
            ..FrameInput::idle()
        },
        COIN_FRAMES,
    ));
    frames.extend(std::iter::repeat_n(FrameInput::idle(), CREDIT_DELAY));

    let start: Switches = [Switch::Start1].into_iter().collect();
    frames.extend(std::iter::repeat_n(
        FrameInput {
            switches: start,
            ..FrameInput::idle()
        },
        START_FRAMES,
    ));

    // Then move: a sweep right and down, then back the other way.
    while frames.len() < FRAMES as usize {
        let (dx, dy) = if frames.len() < 530 { (7, 3) } else { (-5, -9) };
        frames.push(FrameInput {
            dx,
            dy,
            switches: Switches::none(),
        });
    }
    Trace { frames }
}

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn playback_is_deterministic_and_input_reaches_the_machine() {
    let Some(corpus) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let images = build_images(&corpus).expect("build images");

    let idle = Trace::idle(FRAMES as usize);
    let mut player = OurRuntime::new(images.prog.clone(), images.data.clone(), Some(images.mob.clone()));

    let first = player.hashes(&idle, FRAMES).expect("first idle run");
    let second = player.hashes(&idle, FRAMES).expect("second idle run");

    assert_eq!(first.len(), FRAMES as usize, "wrong number of frames");
    // Report the first differing frame rather than letting assert_eq! dump six
    // hundred hashes.
    if let Some(f) = first.iter().zip(&second).position(|(a, b)| a != b) {
        panic!(
            "two identical runs diverged at frame {f} ({:016x} vs {:016x}) — \
             something host-dependent has leaked into the player, and every \
             divergence report built on it would be meaningless",
            first[f], second[f]
        );
    }
    eprintln!(
        "idle:  {FRAMES} frames, first {:016x}, last {:016x}",
        first[0],
        first[FRAMES as usize - 1]
    );

    // Not vacuous: a shorter run must land somewhere else.
    let shorter = player.hashes(&idle, FRAMES / 2).expect("short idle run");
    assert_ne!(
        first[FRAMES as usize - 1],
        shorter[FRAMES as usize / 2 - 1],
        "300 and 600 frames give the same picture, so equality above proves nothing"
    );

    // The input actually arrives.
    let busy = busy_trace();
    let busy_hashes = player.hashes(&busy, FRAMES).expect("busy run");
    let again = player.hashes(&busy, FRAMES).expect("busy run again");
    if let Some(f) = busy_hashes.iter().zip(&again).position(|(a, b)| a != b) {
        panic!("the busy trace is not deterministic — diverged at frame {f}");
    }

    let diverged = first
        .iter()
        .zip(&busy_hashes)
        .position(|(a, b)| a != b)
        .expect(
            "a trace with CoinLeft, Start1 and trackball movement produced \
             exactly the idle picture stream — the player is dropping input",
        );
    eprintln!(
        "busy:  first differs from idle at frame {diverged} \
         ({:016x} vs {:016x})",
        first[diverged], busy_hashes[diverged]
    );
}

/// Our frames carry the cycle count at which they were captured.
///
/// The stamp is what makes a timebase calibration against an external oracle
/// possible at all (`harness.md` §12): comparing frame *N* to frame *N* assumes
/// the two are the same frame, and nothing established that. A stamp turns the
/// offset into something measurable.
///
/// What is asserted here is only our side's arithmetic — monotonic, and one
/// frame period apart, allowing for the overshoot `run_frame` necessarily has.
#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn frames_are_stamped_with_the_cycle_they_were_captured_at() {
    let Some(corpus) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let images = build_images(&corpus).expect("images");
    let mut ours = OurRuntime::new(images.prog, images.data, Some(images.mob));
    let trace = Trace::idle(FRAMES as usize);

    let play = ours.play(&trace, FRAMES).expect("playback");
    assert_eq!(play.stamps.len(), FRAMES as usize, "one stamp per frame");

    const PERIOD: u64 = chill65_runtime::frame::CYCLES_PER_FRAME as u64;

    // A frame ends on the first instruction boundary at or past the deadline,
    // so each stamp sits at or just above (k+1) * PERIOD, and the excess is
    // carried into the next frame rather than reset.
    // The ceiling is derived rather than fitted: a frame can overrun its
    // deadline by less than one instruction, and the longest 6502 instruction
    // is 7 cycles, so the accumulated excess after k frames is under 7k.
    // Measured, it is far tighter — 528 cycles over 600 frames — and that 528
    // is exactly the excess in the attract figure `attract.rs` gates,
    // 12,288,528 against 600 * 20,480 = 12,288,000.
    let mut worst = 0u64;
    for (k, &stamp) in play.stamps.iter().enumerate() {
        let floor = (k as u64 + 1) * PERIOD;
        let ceiling = floor + 7 * (k as u64 + 1);
        assert!(stamp >= floor, "frame {k} stamped {stamp}, below {floor}");
        assert!(stamp < ceiling, "frame {k} stamped {stamp}, at or past {ceiling}");
        worst = worst.max(stamp - floor);
    }
    eprintln!("stamps: worst overshoot {worst} cycles over {FRAMES} frames");

    for pair in play.stamps.windows(2) {
        assert!(pair[1] > pair[0], "stamps must strictly increase: {pair:?}");
    }

    // The last stamp is the run's total, which `attract.rs` gates at
    // 12,288,528 for 600 idle frames.
    assert_eq!(*play.stamps.last().expect("frames"), play.cycles);
}
