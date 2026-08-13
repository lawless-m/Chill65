//! A scripted game, captured to a beam trace, checking itself as it goes.
//!
//! The window is the thing people play, but a window cannot be a test: it
//! needs a person, a display and a pair of eyes. This is the same machine and
//! the same conversion, driven by a script instead of a keyboard, running
//! faster than real time and entirely deterministically — so the whole path
//! from switches to samples can be exercised with nobody watching.
//!
//! # It is its own check
//!
//! A trace file proves nothing by existing. So the capture runs a **second**
//! machine alongside the first, given no input at all, and compares the two
//! frame by frame. Attract mode is deterministic (`attract_sd.rs` proves that),
//! so the idle machine is a control: if the scripted run never diverges from
//! it, the script never reached the game and the command fails rather than
//! writing a file that merely looks plausible.
//!
//! # The script's timings are not invented here
//!
//! Every constant mirrors `chill65-runtime/tests/play_sd.rs`, and
//! `credit_sd.rs` proves the sequence against the machine's own variables:
//! a coin held inside `COIN65.MAC`'s 16-800 ms window buys a credit, select
//! then clears `STRTLOK` from `80` to `40`, and start takes it to `00` —
//! which `AST2RD.MAC:514-517` defines as A GAME IN PROGRESS.

use std::path::Path;

use beam_trace::{write_file, Sample, TraceHeader};
use chill65_runtime::btr0::DEFAULT_EPSILON;

use crate::machine::{offset_onto, SdControls, SdProducer, FRAME_SECONDS, REFRESH_HZ};

/// The left coin, `0800` D2. A set bit means a coin is present.
/// `play_sd.rs::COIN_BIT`.
const COIN_BIT: u8 = 0b100;

/// `play_sd.rs::COIN_FRAME` — when the coin goes in.
const COIN_FRAME: u32 = 60;

/// `play_sd.rs::COIN_HOLD_FRAMES`. `COIN65.MAC` accepts 16-800 ms of coin and
/// rejects anything longer as a stuck mechanism; `credit_sd.rs` measures the
/// window as 2 to 48 frames.
const COIN_HOLD_FRAMES: u32 = 20;

/// When SELECT is pressed. **The start button does nothing before this.**
///
/// `AST2RD.MAC:514-517` documents `STRTLOK`: "80=NO STARTS ALLOWED,
/// 40=SELECT PUSHED...STARTS OK". `AST2RD.MAC:1042` tests it as "LOCKED OUT
/// (NO SELECT YET)?", and `AST2RD.MAC:1087-1098` sets the pushed flag on a
/// *fresh* press — the edge matters, so the button is released again after.
/// A script that inserts a coin and presses start, with no select between,
/// buys a credit and never starts a game.
const SELECT_FRAME: u32 = 120;
const SELECT_HOLD_FRAMES: u32 = 20;

/// After the credit has landed and select has unlocked the start button.
const START_FRAME: u32 = 150;

/// `play_sd.rs::START_HOLD_FRAMES`.
const START_HOLD_FRAMES: u32 = 30;

/// `play_sd.rs::FLY_FRAME` — when the flight controls come alive.
const FLY_FRAME: u32 = 240;

/// Rotate left for a spell, then right, so the ship is visibly steered.
const ROTATE_LEFT: std::ops::Range<u32> = 210..270;
const ROTATE_RIGHT: std::ops::Range<u32> = 270..330;

const PRODUCER_ID: &str = "chill65/space-duel-play";

/// What the script is doing on `frame`: the coin line, and the switches.
fn script(frame: u32) -> (u8, SdControls) {
    let coins = if (COIN_FRAME..COIN_FRAME + COIN_HOLD_FRAMES).contains(&frame) {
        COIN_BIT
    } else {
        0
    };
    let mut c = SdControls::default();
    c.game_select = (SELECT_FRAME..SELECT_FRAME + SELECT_HOLD_FRAMES).contains(&frame);
    c.start = (START_FRAME..START_FRAME + START_HOLD_FRAMES).contains(&frame);
    c.rotate_left = ROTATE_LEFT.contains(&frame);
    c.rotate_right = ROTATE_RIGHT.contains(&frame);
    if frame >= FLY_FRAME {
        c.thrust = true;
        // Five frames on, twenty-five off.
        c.fire = (frame - FLY_FRAME) % 30 < 5;
    }
    (coins, c)
}

/// Play a scripted game and write the beam trace.
pub fn run(corpus: &Path, out: &Path, seconds: f64) -> Result<(), String> {
    let frames = (seconds / FRAME_SECONDS).ceil() as u32;

    let mut played = SdProducer::from_corpus(corpus)?;
    let mut idle = SdProducer::from_corpus(corpus)?;
    played.warm_up()?;
    idle.warm_up()?;

    let mut samples: Vec<Sample> = Vec::new();
    let mut last_t = 0.0f32;
    let mut divergence: Option<u32> = None;

    for frame in 0..frames {
        let (coins, controls) = script(frame);
        let mut frame_samples = played.step_frame(controls.to_input(), coins)?;
        idle.step_frame(SdControls::default().to_input(), 0)?;

        if divergence.is_none() && played.frame_hash() != idle.frame_hash() {
            divergence = Some(frame);
        }

        offset_onto(
            &mut frame_samples,
            f64::from(frame) * FRAME_SECONDS,
            &mut last_t,
        );
        samples.extend(frame_samples);
    }

    let Some(diverged_at) = divergence else {
        return Err(format!(
            "the scripted run never diverged from an idle machine in {frames} \
             frames — the coin and start button did not reach the game, so \
             this trace is attract mode and nothing more"
        ));
    };
    if diverged_at < COIN_FRAME {
        return Err(format!(
            "the two machines diverged at frame {diverged_at}, before the coin \
             went in at {COIN_FRAME} — they were not running the same game, so \
             the comparison proves nothing"
        ));
    }

    write_file(
        out,
        &TraceHeader {
            epoch: 0.0,
            epsilon: DEFAULT_EPSILON,
            nominal_refresh_hz: REFRESH_HZ,
            producer_id: PRODUCER_ID.to_owned(),
        },
        &samples,
    )
    .map_err(|e| format!("{}: {e}", out.display()))?;

    println!("left attract at frame  {diverged_at}");
    println!("frames                 {frames}");
    println!("samples                {}", samples.len());
    println!("trace seconds          {last_t:.4}");
    println!("strokes in last frame  {}", played.segments());
    println!("wrote                  {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_inserts_a_coin_then_selects_then_starts_then_flies() {
        assert_eq!(script(0), (0, SdControls::default()), "idle at first");
        assert_eq!(script(COIN_FRAME).0, COIN_BIT, "the coin goes in");
        assert_eq!(
            script(COIN_FRAME + COIN_HOLD_FRAMES - 1).0,
            COIN_BIT,
            "and is held through the debounce"
        );
        assert_eq!(script(COIN_FRAME + COIN_HOLD_FRAMES).0, 0, "then released");
        assert!(script(SELECT_FRAME).1.game_select, "select unlocks starting");
        assert!(
            !script(SELECT_FRAME + SELECT_HOLD_FRAMES).1.game_select,
            "and is released, because the game edge-detects the press"
        );
        assert!(script(START_FRAME).1.start, "start follows select");
        assert!(!script(START_FRAME - 1).1.start);
        assert!(script(FLY_FRAME).1.thrust, "then it flies");
        assert!(script(FLY_FRAME).1.fire, "firing on the pulse");
        assert!(!script(FLY_FRAME + 10).1.fire, "and off between them");
    }

    #[test]
    fn the_coin_is_held_inside_the_window_and_the_order_is_right() {
        // COIN65.MAC accepts 16-800 ms of coin present and rejects anything
        // longer as a stuck mechanism. credit_sd.rs measures that as 2 to 48
        // frames here, so both ends of this are mistakes worth failing for.
        assert!(
            (2..=48).contains(&COIN_HOLD_FRAMES),
            "a {COIN_HOLD_FRAMES}-frame coin falls outside the 16-800 ms window"
        );
        assert!(
            SELECT_FRAME >= COIN_FRAME + COIN_HOLD_FRAMES - 1,
            "select must not come before the credit"
        );
        assert!(
            START_FRAME >= SELECT_FRAME + SELECT_HOLD_FRAMES,
            "start is locked out until select has been pressed and released"
        );
    }
}
