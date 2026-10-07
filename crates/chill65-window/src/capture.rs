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
//! frame by frame. Attract mode is deterministic (`attract_sd.rs` and
//! `attract_te.rs` prove that for their games), so the idle machine is a
//! control: if the scripted run never diverges from it, the script never
//! reached the game and the command fails rather than writing a file that
//! merely looks plausible.

use std::path::Path;

use beam_trace::{write_file, Sample, TraceHeader};
use chill65_runtime::btr0::DEFAULT_EPSILON;

use crate::machine::{offset_onto, Controls, Game, Producer, SdControls, TeControls};

/// Space Duel's script.
///
/// Every constant mirrors `chill65-runtime/tests/play_sd.rs`, and `credit_sd.rs`
/// proves the sequence against the machine's own variables: a coin held inside
/// `COIN65.MAC`'s 16-800 ms window buys a credit, select then clears `STRTLOK`
/// from `80` to `40`, and start takes it to `00` — which `AST2RD.MAC:514-517`
/// defines as A GAME IN PROGRESS.
mod space_duel {
    use super::SdControls;

    /// `play_sd.rs::COIN_FRAME` — when the coin goes in.
    pub const COIN_FRAME: u32 = 60;

    /// `play_sd.rs::COIN_HOLD_FRAMES`. `COIN65.MAC` accepts 16-800 ms of coin
    /// and rejects anything longer as a stuck mechanism; `credit_sd.rs`
    /// measures the window as 2 to 48 frames.
    pub const COIN_HOLD_FRAMES: u32 = 20;

    /// When SELECT is pressed. **The start button does nothing before this.**
    ///
    /// `AST2RD.MAC:514-517` documents `STRTLOK`: "80=NO STARTS ALLOWED,
    /// 40=SELECT PUSHED...STARTS OK". `AST2RD.MAC:1042` tests it as "LOCKED
    /// OUT (NO SELECT YET)?", and `AST2RD.MAC:1087-1098` sets the pushed flag
    /// on a *fresh* press — the edge matters, so the button is released again
    /// after. A script that inserts a coin and presses start, with no select
    /// between, buys a credit and never starts a game.
    pub const SELECT_FRAME: u32 = 120;
    pub const SELECT_HOLD_FRAMES: u32 = 20;

    /// After the credit has landed and select has unlocked the start button.
    pub const START_FRAME: u32 = 150;

    /// `play_sd.rs::START_HOLD_FRAMES`.
    pub const START_HOLD_FRAMES: u32 = 30;

    /// `play_sd.rs::FLY_FRAME` — when the flight controls come alive.
    pub const FLY_FRAME: u32 = 240;

    /// Rotate left for a spell, then right, so the ship is visibly steered.
    pub const ROTATE_LEFT: std::ops::Range<u32> = 210..270;
    pub const ROTATE_RIGHT: std::ops::Range<u32> = 270..330;

    pub fn script(frame: u32) -> (bool, SdControls) {
        let coin = (COIN_FRAME..COIN_FRAME + COIN_HOLD_FRAMES).contains(&frame);
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
        (coin, c)
    }
}

/// Tempest's script.
///
/// A different panel and a slower frame, so none of Space Duel's numbers carry
/// over. What is the same is `COIN65.MAC`, which is byte-identical between the
/// corpora and accepts 16-800 ms of coin present; at 36 ms a frame that is 1 to
/// 22 frames rather than 2 to 48.
///
/// There is no SELECT here. `ALEXEC.MAC:132-152`'s `PROCRE` reads the two start
/// buttons out of `SWFINA` — a latched buffer it clears as it reads, so the
/// press is an edge and a held button starts one game — and takes a credit for
/// whichever it finds. `ALEXEC.MAC:244-248` grants two credits outright when
/// `$CMODE & 3` is zero, so on the option bytes this frontend runs the coin is
/// not what unlocks the game; it is scripted anyway, because a capture that
/// never worked the mech would not exercise it.
mod tempest {
    use super::TeControls;

    pub const COIN_FRAME: u32 = 30;

    /// Inside `COIN65.MAC`'s window at this board's frame rate: 360 ms.
    pub const COIN_HOLD_FRAMES: u32 = 10;

    /// One player. Latched on the edge, so a short press is enough.
    pub const START_FRAME: u32 = 60;
    pub const START_HOLD_FRAMES: u32 = 5;

    /// The game opens on a level chooser; fire accepts what the spinner has
    /// landed on and drops the claw onto the web.
    pub const FIRE_TO_PLAY_FRAME: u32 = 90;
    pub const FIRE_TO_PLAY_HOLD: u32 = 5;

    /// When the claw starts moving, and which way, so the web is visibly
    /// worked from one side to the other.
    pub const SPIN_LEFT: std::ops::Range<u32> = 120..170;
    pub const SPIN_RIGHT: std::ops::Range<u32> = 170..220;

    /// Once the claw is on the web, shooting.
    pub const SHOOT_FRAME: u32 = 120;

    /// A superzapper, once, late enough that there is something to zap.
    pub const ZAP_FRAME: u32 = 230;
    pub const ZAP_HOLD_FRAMES: u32 = 5;

    pub fn script(frame: u32) -> (bool, TeControls) {
        let coin = (COIN_FRAME..COIN_FRAME + COIN_HOLD_FRAMES).contains(&frame);
        let mut c = TeControls::default();
        c.start1 = (START_FRAME..START_FRAME + START_HOLD_FRAMES).contains(&frame);
        c.rotate_left = SPIN_LEFT.contains(&frame);
        c.rotate_right = SPIN_RIGHT.contains(&frame);
        c.superzapper = (ZAP_FRAME..ZAP_FRAME + ZAP_HOLD_FRAMES).contains(&frame);
        c.fire = (FIRE_TO_PLAY_FRAME..FIRE_TO_PLAY_FRAME + FIRE_TO_PLAY_HOLD).contains(&frame)
            // Three frames on, nine off.
            || (frame >= SHOOT_FRAME && (frame - SHOOT_FRAME) % 12 < 3);
        (coin, c)
    }
}

/// What the script is doing on `frame`: the coin line, and the switches.
fn script(game: Game, frame: u32) -> (bool, Controls) {
    match game {
        Game::SpaceDuel => {
            let (coin, c) = space_duel::script(frame);
            (coin, Controls::Sd(c))
        }
        Game::Tempest => {
            let (coin, c) = tempest::script(frame);
            (coin, Controls::Te(c))
        }
    }
}

/// The first frame on which the script touches anything.
///
/// A run that leaves attract mode *before* this was never running the same game
/// as its control, so the comparison would prove nothing.
fn first_input_frame(game: Game) -> u32 {
    match game {
        Game::SpaceDuel => space_duel::COIN_FRAME,
        Game::Tempest => tempest::COIN_FRAME,
    }
}

fn producer_id(game: Game) -> &'static str {
    match game {
        Game::SpaceDuel => "chill65/space-duel-play",
        Game::Tempest => "chill65/tempest-play",
    }
}

/// Play a scripted game and write the beam trace.
pub fn run(corpus: &Path, out: &Path, seconds: f64) -> Result<(), String> {
    let game = Game::detect(corpus)?;
    let frame_seconds = game.frame_seconds();
    let frames = (seconds / frame_seconds).ceil() as u32;

    let mut played = Producer::from_corpus(game, corpus)?;
    let mut idle = Producer::from_corpus(game, corpus)?;
    played.warm_up()?;
    idle.warm_up()?;

    let idle_controls = Controls::new(game);
    let mut samples: Vec<Sample> = Vec::new();
    let mut last_t = 0.0f32;
    let mut divergence: Option<u32> = None;

    for frame in 0..frames {
        let (coin, controls) = script(game, frame);
        let mut frame_samples = played.step_frame(&controls, coin)?;
        idle.step_frame(&idle_controls, false)?;

        if divergence.is_none() && played.frame_hash() != idle.frame_hash() {
            divergence = Some(frame);
        }

        offset_onto(&mut frame_samples, f64::from(frame) * frame_seconds, &mut last_t);
        samples.extend(frame_samples);
    }

    let first_input = first_input_frame(game);
    let Some(diverged_at) = divergence else {
        return Err(format!(
            "the scripted run never diverged from an idle machine in {frames} \
             frames — the panel did not reach the game, so this trace is \
             attract mode and nothing more"
        ));
    };
    if diverged_at < first_input {
        return Err(format!(
            "the two machines diverged at frame {diverged_at}, before the \
             script touched anything at {first_input} — they were not running \
             the same game, so the comparison proves nothing"
        ));
    }

    write_file(
        out,
        &TraceHeader {
            epoch: 0.0,
            epsilon: DEFAULT_EPSILON,
            nominal_refresh_hz: game.refresh_hz(),
            producer_id: producer_id(game).to_owned(),
        },
        &samples,
    )
    .map_err(|e| format!("{}: {e}", out.display()))?;

    println!("game                   {}", game.title());
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
    fn the_space_duel_script_coins_then_selects_then_starts_then_flies() {
        use space_duel::{
            COIN_FRAME, COIN_HOLD_FRAMES, FLY_FRAME, SELECT_FRAME, SELECT_HOLD_FRAMES, START_FRAME,
        };
        assert_eq!(
            script(Game::SpaceDuel, 0),
            (false, Controls::new(Game::SpaceDuel)),
            "idle at first"
        );
        assert!(space_duel::script(COIN_FRAME).0, "the coin goes in");
        assert!(
            space_duel::script(COIN_FRAME + COIN_HOLD_FRAMES - 1).0,
            "and is held through the debounce"
        );
        assert!(
            !space_duel::script(COIN_FRAME + COIN_HOLD_FRAMES).0,
            "then released"
        );
        assert!(
            space_duel::script(SELECT_FRAME).1.game_select,
            "select unlocks starting"
        );
        assert!(
            !space_duel::script(SELECT_FRAME + SELECT_HOLD_FRAMES).1.game_select,
            "and is released, because the game edge-detects the press"
        );
        assert!(space_duel::script(START_FRAME).1.start, "start follows select");
        assert!(!space_duel::script(START_FRAME - 1).1.start);
        assert!(space_duel::script(FLY_FRAME).1.thrust, "then it flies");
        assert!(space_duel::script(FLY_FRAME).1.fire, "firing on the pulse");
        assert!(!space_duel::script(FLY_FRAME + 10).1.fire, "and off between");
    }

    #[test]
    fn the_tempest_script_coins_then_starts_then_works_the_web() {
        use tempest::{
            COIN_FRAME, COIN_HOLD_FRAMES, FIRE_TO_PLAY_FRAME, SHOOT_FRAME, SPIN_LEFT, SPIN_RIGHT,
            START_FRAME, START_HOLD_FRAMES, ZAP_FRAME,
        };
        assert_eq!(
            script(Game::Tempest, 0),
            (false, Controls::new(Game::Tempest)),
            "idle at first"
        );
        assert!(tempest::script(COIN_FRAME).0, "the coin goes in");
        assert!(
            !tempest::script(COIN_FRAME + COIN_HOLD_FRAMES).0,
            "and comes out again"
        );
        assert!(tempest::script(START_FRAME).1.start1, "one player starts");
        assert!(
            !tempest::script(START_FRAME + START_HOLD_FRAMES).1.start1,
            "and the button is released, because PROCRE reads an edge"
        );
        assert!(
            tempest::script(FIRE_TO_PLAY_FRAME).1.fire,
            "fire takes the chosen level"
        );
        assert!(
            tempest::script(SPIN_LEFT.start).1.rotate_left,
            "then the claw is walked one way"
        );
        assert!(
            tempest::script(SPIN_RIGHT.start).1.rotate_right,
            "and back the other"
        );
        assert!(
            !tempest::script(SPIN_RIGHT.start).1.rotate_left,
            "never both at once"
        );
        assert!(tempest::script(ZAP_FRAME).1.superzapper, "one zap");
        assert!(tempest::script(SHOOT_FRAME).1.fire, "shooting on the pulse");
        assert!(!tempest::script(SHOOT_FRAME + 5).1.fire, "and off between");
    }

    /// `COIN65.MAC` is the same file in both corpora, so the same milliseconds
    /// bound both coins — and the two boards count them in frames of very
    /// different lengths.
    #[test]
    fn each_coin_is_held_inside_the_mechs_window_and_the_order_is_right() {
        for (game, hold) in [
            (Game::SpaceDuel, space_duel::COIN_HOLD_FRAMES),
            (Game::Tempest, tempest::COIN_HOLD_FRAMES),
        ] {
            let ms = f64::from(hold) * game.frame_seconds() * 1000.0;
            assert!(
                (16.0..=800.0).contains(&ms),
                "{}: a {hold}-frame coin is {ms:.0} ms, outside COIN65's window",
                game.title()
            );
        }

        assert!(
            space_duel::SELECT_FRAME
                >= space_duel::COIN_FRAME + space_duel::COIN_HOLD_FRAMES - 1,
            "select must not come before the credit"
        );
        assert!(
            space_duel::START_FRAME >= space_duel::SELECT_FRAME + space_duel::SELECT_HOLD_FRAMES,
            "start is locked out until select has been pressed and released"
        );
        assert!(
            tempest::START_FRAME >= tempest::COIN_FRAME + tempest::COIN_HOLD_FRAMES,
            "start must not come before the credit"
        );
        assert!(
            tempest::FIRE_TO_PLAY_FRAME >= tempest::START_FRAME + tempest::START_HOLD_FRAMES,
            "the level chooser is not there until the game has been started"
        );
    }
}
