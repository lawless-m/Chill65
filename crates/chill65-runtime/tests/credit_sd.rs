//! A coin buys a credit, and the start button stops being locked out.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/space-duel \
//!   cargo test -p chill65-runtime --test credit_sd -- --ignored --nocapture
//! ```
//!
//! # Why this test exists
//!
//! `play_sd.rs` shows the panel reaches the machine. That is not the same as
//! playing: a coin has to be *accepted*, and until it is, `AST2RD.MAC`'s
//! `STRTLOK` stays at `80` — "NO STARTS ALLOWED" — and the start button is
//! correctly ignored. This reads the game's own variables through the link's
//! symbol table and asserts the credit actually lands.
//!
//! # A coin can be held too long
//!
//! `COIN65.MAC`'s commentary (the block from line ~255) is unusually explicit:
//!
//! > a valid coin is defined as between 16 and 800 ms of coin present,
//! > preceded and followed by 33 ms of coin absent ... A count from 27-31
//! > (less than 16-20 ms) is too short. A count of 0 (more than 800 ms) is too
//! > long. Both of these cases are simply reset to 31.
//!
//! So the coin line has a **window**, not a threshold: roughly 1 to 49 frames
//! at 61.5 Hz. Holding it down longer is a stuck mechanism, not a generous
//! player, and the game rejects it — which is exactly what an earlier 72-frame
//! hold was doing. The sweep below measures the window rather than assuming
//! it, and [`COIN_HOLD_FRAMES`] is picked from the middle of what works.

mod common;

use chill65_runtime::sd::{run_frame_sd, SdInput, SdMachine};
use chill65_runtime::Cpu;

use common::{build_sd_program, build_sd_ship, corpus, SdBuild, SD_PROG_LEN};

/// Frames of attract before the coin goes in.
const WARMUP: u32 = 300;

/// The left coin, `0800` D2. A set bit means a coin is present.
pub const COIN_BIT: u8 = 0b100;

/// How long to hold the coin line: inside `COIN65.MAC`'s 16-800 ms window,
/// with room either side. Proved by the sweep in this file.
pub const COIN_HOLD_FRAMES: u32 = 20;

/// `STRTLOK` while starting is refused — `AST2RD.MAC:514-517`.
const NO_STARTS: u8 = 0x80;

/// Look a game variable up by name. MACRO-11 truncates to six characters, so
/// `STRTLOK` is found as `STRTLO`.
fn addr(build: &SdBuild, name: &str) -> u16 {
    build
        .symbol(name)
        .unwrap_or_else(|| panic!("{name} is not in the symbol table"))
}

struct Machine {
    cpu: Cpu,
    m: SdMachine,
}

impl Machine {
    fn boot(ship: &[u8], image: &[u8]) -> Self {
        let mut m = SdMachine::new();
        m.load_roms(ship, image).expect("load");
        m.set_options(0, 0);
        m.self_test = false;
        m.input = SdInput::upright();
        let mut cpu = Cpu::new();
        cpu.reset(&mut m);
        let mut machine = Machine { cpu, m };
        for _ in 0..WARMUP {
            machine.frame(0, SdInput::upright());
        }
        machine
    }

    fn frame(&mut self, coins: u8, input: SdInput) {
        self.m.coins = coins;
        self.m.input = input;
        run_frame_sd(&mut self.cpu, &mut self.m).expect("a frame");
    }

    fn peek(&self, at: u16) -> u8 {
        self.m.ram[at as usize]
    }
}

/// Insert one coin held for `hold` frames, then wait, and report the credit.
fn credits_after(ship: &[u8], image: &[u8], crdt: u16, hold: u32) -> u8 {
    let mut machine = Machine::boot(ship, image);
    for _ in 0..hold {
        machine.frame(COIN_BIT, SdInput::upright());
    }
    // The coin must be *absent* for a spell before it counts — the routine
    // waits for VALID-LOW, then runs a post-coin slam timer of about half a
    // second before the credit lands.
    for _ in 0..120 {
        machine.frame(0, SdInput::upright());
    }
    machine.peek(crdt)
}

#[test]
#[ignore = "needs CHILL65_CORPUS"]
fn a_coin_held_within_the_window_buys_a_credit() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let build = build_sd_program(&c);
    let crdt = addr(&build, "$$CRDT");
    let strtlok = addr(&build, "STRTLO");
    let ship = build_sd_ship(&c);
    let image = build.bytes(0x0000, SD_PROG_LEN);
    println!("\n$$CRDT at {crdt:04X}, STRTLOK at {strtlok:04X}");

    // Measure the window rather than believing the commentary.
    println!("\n  hold (frames)   ms   credits");
    let mut accepted = Vec::new();
    for hold in [2u32, 5, 10, 20, 30, 40, 48, 60, 72] {
        let credits = credits_after(&ship, &image, crdt, hold);
        let ms = f64::from(hold) * 1000.0 / 61.5234;
        println!("  {hold:>13} {ms:>5.0}   {credits}");
        if credits > 0 {
            accepted.push(hold);
        }
    }
    println!();

    assert!(
        !accepted.is_empty(),
        "no coin hold between 2 and 72 frames bought a credit — the coin path \
         is broken, or the machine needs configuration this test does not do"
    );
    assert!(
        accepted.contains(&COIN_HOLD_FRAMES),
        "COIN_HOLD_FRAMES is {COIN_HOLD_FRAMES}, which is not among the holds \
         that worked: {accepted:?}"
    );
    assert!(
        !accepted.contains(&72),
        "a 72-frame coin is over COIN65.MAC's 800 ms ceiling and should have \
         been rejected as a stuck mechanism"
    );

    // Then the whole sequence a player performs, with the game's own lockout
    // byte as the witness. `AST2RD.MAC:514-517` gives its three values:
    // 80 = no starts allowed, 40 = select pushed so starts are ok, and
    // 0 = A GAME IN PROGRESS.
    let mut machine = Machine::boot(&ship, &image);
    assert_eq!(
        machine.peek(strtlok),
        NO_STARTS,
        "with no credit, starts must be locked out"
    );

    for _ in 0..COIN_HOLD_FRAMES {
        machine.frame(COIN_BIT, SdInput::upright());
    }
    for _ in 0..120 {
        machine.frame(0, SdInput::upright());
    }
    let credits = machine.peek(crdt);
    println!("after a coin:   credits {credits}, STRTLOK {:02X}", machine.peek(strtlok));
    assert!(credits > 0, "the coin bought nothing");

    // Select, pressed and released — `AST2RD.MAC:1087-1098` edge-detects it.
    let mut select = SdInput::upright();
    select.game_select = true;
    for _ in 0..10 {
        machine.frame(0, select);
    }
    for _ in 0..10 {
        machine.frame(0, SdInput::upright());
    }
    let after_select = machine.peek(strtlok);
    println!("after select:   credits {}, STRTLOK {after_select:02X}", machine.peek(crdt));
    assert_ne!(
        after_select, NO_STARTS,
        "select did not unlock starting — `BIT STRTLOK / BMI` at \
         AST2RD.MAC:1042 will keep ignoring the start button"
    );

    // And start.
    let mut start = SdInput::upright();
    start.start = true;
    for _ in 0..10 {
        machine.frame(0, start);
    }
    for _ in 0..30 {
        machine.frame(0, SdInput::upright());
    }
    let playing = machine.peek(strtlok);
    println!(
        "after start:    credits {}, STRTLOK {playing:02X}\n",
        machine.peek(crdt)
    );
    assert_eq!(
        playing, 0,
        "STRTLOK is {playing:02X}; AST2RD.MAC:514-517 says 0 is A GAME IN \
         PROGRESS, so the game did not start"
    );
}
