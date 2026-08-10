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
    let mut player = OurRuntime::new(images.prog.clone(), images.data.clone());

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
