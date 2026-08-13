//! Space Duel, played: a coin, a start button, and the controls steering it.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/space-duel \
//!   cargo test -p chill65-runtime --test play_sd -- --ignored --nocapture
//! ```
//!
//! # What this proves, and how
//!
//! `attract_sd.rs` shows the machine runs and keeps drawing. That is not the
//! same as *playable*: attract mode would do exactly the same with the panel
//! disconnected. What playable means is that the switches reach the game and
//! change what it does.
//!
//! Nobody can sit and watch, so the evidence is differential. Two machines run
//! from identical state; one is left alone and one is played. Attract mode is
//! deterministic (`attract_sd.rs` proves that separately), so the idle machine
//! is a control: while both are idle their frame hashes agree exactly, and the
//! frame where they stop agreeing is the frame the input took effect. A third
//! run adds thrust, rotation and fire on top of the coin and start, and must
//! diverge from the second — otherwise the credit was accepted but the flight
//! controls go nowhere.
//!
//! # The coin has a window, and so does the start button
//!
//! A coin is not a button. `COIN65.MAC`'s commentary is explicit: "a valid coin
//! is defined as between 16 and 800 ms of coin present, preceded and followed
//! by 33 ms of coin absent", and "a count of 0 (more than 800 ms) is too long"
//! — a held line is a stuck mechanism, and the game rejects it. `credit_sd.rs`
//! measures that window on this machine and proves a credit lands.
//!
//! Starting then needs SELECT first: `AST2RD.MAC:1042` branches past the start
//! check while `STRTLOK` is `80`, and only a fresh select press sets `40`.
//! Coin, select, start — in that order, with pauses.

mod common;

use chill65_runtime::sd::{run_frame_sd, SdInput, SdMachine};
use chill65_runtime::{Cpu, CpuError};

use common::{build_sd_program, build_sd_ship, corpus, SD_PROG_LEN};

/// Frames of attract before the script starts. Drawing begins around frame 99
/// (`boot_sd.rs`), so this is well clear of the power-on self-test.
const WARMUP: u32 = 300;

/// Frames of script, after the warmup.
const FRAMES: u32 = 900;

/// The left coin, `0800` D2 (`AS2DEC.MAC:11`). A set bit in `SdMachine::coins`
/// means a coin is present; the low-true inversion happens on the bus read.
pub const COIN_BIT: u8 = 0b100;

/// When the coin goes in, relative to the end of the warmup.
pub const COIN_FRAME: u32 = 60;

/// How long the coin line is held. See the module note: the valid window is
/// 16-800 ms, and `credit_sd.rs` measures it as 2 to 48 frames on this
/// machine. Twenty sits comfortably inside.
pub const COIN_HOLD_FRAMES: u32 = 20;

/// When SELECT is pressed. The start button is ignored before this:
/// `AST2RD.MAC:1042` tests `STRTLOK` and branches past the start check, and
/// only a fresh select press sets the `40` "starts ok" flag
/// (`AST2RD.MAC:1087-1098`). Late enough that the credit has landed — the
/// post-coin slam timer runs about 30 frames after the coin goes away.
pub const SELECT_FRAME: u32 = 120;
pub const SELECT_HOLD_FRAMES: u32 = 20;

/// When the start button goes down — after the credit and after select.
pub const START_FRAME: u32 = 150;

/// How long start is held.
pub const START_HOLD_FRAMES: u32 = 30;

/// When the flight controls come alive, in the third run.
pub const FLY_FRAME: u32 = 240;

/// What the player is doing on a given frame.
type Script = fn(u32) -> (u8, SdInput);

/// Nothing at all: the control.
fn idle(_frame: u32) -> (u8, SdInput) {
    (0, SdInput::upright())
}

/// A coin, select, and start — the sequence a player actually performs.
fn coin_and_start(frame: u32) -> (u8, SdInput) {
    let coins = if (COIN_FRAME..COIN_FRAME + COIN_HOLD_FRAMES).contains(&frame) {
        COIN_BIT
    } else {
        0
    };
    let mut input = SdInput::upright();
    input.game_select = (SELECT_FRAME..SELECT_FRAME + SELECT_HOLD_FRAMES).contains(&frame);
    input.start = (START_FRAME..START_FRAME + START_HOLD_FRAMES).contains(&frame);
    (coins, input)
}

/// The same, and then actually flying: thrust and rotation held, fire pulsed.
fn playing(frame: u32) -> (u8, SdInput) {
    let (coins, mut input) = coin_and_start(frame);
    if frame >= FLY_FRAME {
        input.thrust = true;
        input.rotate_left = true;
        input.fire = (frame - FLY_FRAME) % 30 < 5;
    }
    (coins, input)
}

struct Run {
    hashes: Vec<u64>,
    /// Strokes drawn per frame. Attract is a steady 322; a different screen
    /// has a different profile, which is evidence a hash alone cannot give.
    segments: Vec<usize>,
    drew: usize,
}

/// Warm up, then run `script` for [`FRAMES`] frames, recording the picture.
fn run(ship: &[u8], image: &[u8], script: Script) -> Result<Run, CpuError> {
    let mut m = SdMachine::new();
    m.load_roms(ship, image).expect("load");
    m.set_options(0, 0);
    m.self_test = false;
    m.input = SdInput::upright();

    let mut cpu = Cpu::new();
    cpu.reset(&mut m);
    for _ in 0..WARMUP {
        run_frame_sd(&mut cpu, &mut m)?;
    }

    let mut hashes = Vec::with_capacity(FRAMES as usize);
    let mut segments = Vec::with_capacity(FRAMES as usize);
    let mut drew = 0usize;
    for frame in 0..FRAMES {
        let (coins, input) = script(frame);
        m.coins = coins;
        m.input = input;
        let stats = run_frame_sd(&mut cpu, &mut m)?;

        assert_eq!(stats.irqs_raised, 4, "frame {frame}: interrupt cadence");
        assert!(!m.watchdog_expired, "frame {frame}: the machine wedged");
        assert!(m.vg_fault.is_none(), "frame {frame}: {:?}", m.vg_fault);
        if !m.segments.is_empty() {
            drew += 1;
        }
        segments.push(m.segments.len());
        hashes.push(m.frame_hash());
    }
    Ok(Run {
        hashes,
        segments,
        drew,
    })
}

/// The first frame at which two runs disagree.
fn diverges_at(a: &[u64], b: &[u64]) -> Option<u32> {
    a.iter().zip(b).position(|(x, y)| x != y).map(|i| i as u32)
}

#[test]
#[ignore = "needs CHILL65_CORPUS"]
fn a_coin_and_the_controls_change_what_the_machine_does() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let ship = build_sd_ship(&c);
    let image = build_sd_program(&c).bytes(0x0000, SD_PROG_LEN);

    let control = run(&ship, &image, idle).expect("idle run");
    let started = run(&ship, &image, coin_and_start).expect("coin and start");
    let played = run(&ship, &image, playing).expect("playing");

    let start_div = diverges_at(&control.hashes, &started.hashes);
    let fly_div = diverges_at(&started.hashes, &played.hashes);

    println!("\n{FRAMES} frames after a {WARMUP}-frame warmup");
    println!("  coin  frame {COIN_FRAME}, held {COIN_HOLD_FRAMES}, bit {COIN_BIT:#05b}");
    println!("  sel   frame {SELECT_FRAME}, held {SELECT_HOLD_FRAMES}");
    println!("  start frame {START_FRAME}, held {START_HOLD_FRAMES}");
    println!("  fly   frame {FLY_FRAME} onward");
    println!("  idle vs coin+start diverges at {start_div:?}");
    println!("  coin+start vs playing diverges at {fly_div:?}");
    println!(
        "  frames that drew: idle {}, started {}, played {}",
        control.drew, started.drew, played.drew
    );
    let profile = |r: &Run| {
        let tail = &r.segments[(FRAMES - 200) as usize..];
        (
            *tail.iter().min().unwrap(),
            *tail.iter().max().unwrap(),
            tail.iter().sum::<usize>() / tail.len(),
        )
    };
    println!(
        "  strokes per frame over the last 200 (min/max/mean): \
         idle {:?}, played {:?}\n",
        profile(&control),
        profile(&played)
    );

    let start_div = start_div.unwrap_or_else(|| {
        panic!(
            "the coin and start button changed nothing in {FRAMES} frames — \
             the machine is not playable"
        )
    });
    assert!(
        start_div >= COIN_FRAME,
        "the runs diverged at {start_div}, before the coin went in at \
         {COIN_FRAME} — attract mode is not deterministic and this test proves \
         nothing"
    );

    // Not a blip: the machine is somewhere else now and stays there.
    let tail = (FRAMES - 200) as usize;
    let same_late = control.hashes[tail..]
        .iter()
        .zip(&started.hashes[tail..])
        .filter(|(a, b)| a == b)
        .count();
    assert!(
        same_late * 4 < 200,
        "the played machine drifted back into step with the idle one on \
         {same_late} of the last 200 frames"
    );

    let fly_div = fly_div.unwrap_or_else(|| {
        panic!(
            "thrust, rotation and fire changed nothing — the credit was taken \
             but the controls go nowhere"
        )
    });
    assert!(
        fly_div >= FLY_FRAME,
        "the flight controls appeared to act at {fly_div}, before they were \
         touched at {FLY_FRAME}"
    );

    // It has to keep drawing while being played, or "playable" means nothing.
    let floor = (FRAMES as usize * 9) / 10;
    assert!(played.drew > floor, "only {} frames drew", played.drew);

    // And it must be drawing something *else*. Equal hashes already imply
    // equal stroke counts, so this only adds evidence where it differs: a
    // steady title screen and a game in progress do not draw alike.
    let tail = (FRAMES - 200) as usize;
    let different_shape = control.segments[tail..]
        .iter()
        .zip(&played.segments[tail..])
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        different_shape > 100,
        "over the last 200 frames the played machine drew the same number of \
         strokes as the idle one on {} of them — the picture may have changed \
         but the screen has not",
        200 - different_shape
    );
}
