//! Space Duel boot smoke test: assemble the real ROMs and run the machine.
//!
//! The counterpart of `boot.rs`, which does the same for Crystal Castles.
//!
//! # Why this is gated
//!
//! The game source is not in this repository and never will be — plan §9: we
//! ship the toolchain, the user supplies their own source. So this test is
//! `#[ignore]`d and gated on `CHILL65_CORPUS`, and the default `cargo test`
//! stays green with no corpus present.
//!
//! The images it builds are **build artefacts containing game-derived bytes**.
//! They are held in memory and must never be committed.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/space-duel \
//!   cargo test -p chill65-runtime --test boot_sd -- --ignored --nocapture
//! ```

mod common;

use chill65_runtime::sd::{run_frame_sd, SdMachine};
use chill65_runtime::{Bus, Cpu};

use common::{build_sd_program, build_sd_ship, corpus, SD_PROG_LEN};

/// How many frames to run.
///
/// Measured on this model, not guessed:
///
/// - **frame 0** — the first interrupt is taken. `ASTRD2.MAC`'s `PWRON`
///   reaches its `CLI` almost at once, unlike Crystal Castles, which masks
///   interrupts through a long power-on self-test and takes until frame 353.
/// - **frame 98** — the first `GOADD` strobe.
/// - **frame 99** — the first non-empty segment list.
///
/// So there *is* a quiet stretch, about 1.6 seconds of it, before anything is
/// drawn. A budget that stopped inside it would see a blank screen and a
/// healthy watchdog and be unable to tell that from a wedge.
///
/// 200 frames leaves roughly 100 frames of steady state after the first draw —
/// enough for the attract sequence to animate several times over, and enough
/// that "one `GOADD` per frame" is a meaningful thing to assert.
const SMOKE_FRAMES: u32 = 200;

/// Option bytes, both zero.
///
/// `AS2DEC.MAC:149-158` lays out `OPTN1` as bonus level, language, difficulty
/// and lives, and `OPTN2` as the coin-routine switches. Zero is what an
/// undriven switch bank reads and what [`chill65_runtime::Pokey`] gives by
/// default, so it is the honest starting point rather than a tuned one.
///
/// The convenient consequence is free play — `COIN65.MAC:255` documents coin
/// mode 0 as "FREE PLAY" — which is what lets an attract-mode smoke test run
/// without simulating a coin.
const OPTN1: u8 = 0x00;
const OPTN2: u8 = 0x00;

#[test]
#[ignore = "needs CHILL65_CORPUS"]
fn the_real_roms_boot_and_run() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let ship = build_sd_ship(&c);
    let program = build_sd_program(&c);
    let image = program.bytes(0x0000, SD_PROG_LEN);

    let mut m = SdMachine::new();
    m.load_roms(&ship, &image).expect("load");
    m.set_options(OPTN1, OPTN2);
    // Self-test switch idle. `AS2DEC.MAC:9`: 0 = on, so false is "not
    // testing"; `selftest_sd.rs` is the test that holds it the other way.
    m.self_test = false;

    // Before running a single instruction: the vectors must land in program
    // ROM. This proves the top-page mirror and the link at once. Addresses are
    // printed and compared as ranges — no oracle bytes become constants here.
    let reset = m.read_u16(0xFFFC);
    let irq = m.read_u16(0xFFFE);
    println!("reset vector {reset:04X}, IRQ vector {irq:04X}");
    assert!(
        (0x4000..=0x8FFF).contains(&reset),
        "reset vector {reset:04X} is outside program ROM"
    );
    assert!(
        (0x4000..=0x8FFF).contains(&irq),
        "IRQ vector {irq:04X} is outside program ROM"
    );

    let mut cpu = Cpu::new();
    cpu.reset(&mut m);
    assert_eq!(cpu.pc, reset, "the CPU starts at the reset vector");

    let mut irqs_taken = 0u64;
    let mut first_irq = None;
    let mut first_go = None;
    let mut first_draw = None;
    let mut hashes = Vec::new();

    for frame in 0..SMOKE_FRAMES {
        let stats = run_frame_sd(&mut cpu, &mut m)
            .unwrap_or_else(|e| panic!("CPU error on frame {frame}: {e:?}"));
        irqs_taken += stats.irqs_taken as u64;

        if first_irq.is_none() && stats.irqs_taken > 0 {
            first_irq = Some(frame);
        }
        if first_go.is_none() && m.vg_go_strobes > 0 {
            first_go = Some(frame);
        }
        if first_draw.is_none() && !m.segments.is_empty() {
            first_draw = Some(frame);
        }
        hashes.push(m.frame_hash());

        assert_eq!(stats.irqs_raised, 4, "frame {frame}");
        assert!(
            !m.watchdog_expired,
            "the watchdog expired on frame {frame}; the machine wedged"
        );
        assert!(
            m.vg_fault.is_none(),
            "vector generator fault on frame {frame}: {:?}",
            m.vg_fault
        );
    }

    println!(
        "{SMOKE_FRAMES} frames, {} cycles\n  \
         first IRQ taken   frame {first_irq:?}\n  \
         first GOADD       frame {first_go:?}\n  \
         first segments    frame {first_draw:?}\n  \
         IRQs taken        {irqs_taken}\n  \
         GO strobes        {}\n  \
         watchdog strobes  {}\n  \
         segments now      {}\n  \
         distinct hashes   {}",
        m.cycles,
        m.vg_go_strobes,
        m.watchdog_strobes,
        m.segments.len(),
        {
            let mut h = hashes.clone();
            h.sort_unstable();
            h.dedup();
            h.len()
        }
    );

    assert!(
        m.cycles >= SMOKE_FRAMES as u64 * 24_576,
        "ran {} cycles, expected at least one frame's worth each",
        m.cycles
    );
    assert!(irqs_taken > 0, "no interrupt was ever taken");
    assert!(
        m.watchdog_strobes > 0,
        "the game never fed the watchdog; ASTRD2.MAC:728 does so once a frame"
    );
    assert!(
        m.vg_go_strobes > 0,
        "the game never started the vector generator"
    );
    assert!(
        !m.segments.is_empty(),
        "nothing was drawn by frame {SMOKE_FRAMES}"
    );

    // Once the main loop is running it strobes GO once per `SYNC`, which is
    // once per frame (`ASTRD2.MAC:744-747`). Allowing half that is slack for
    // the exact frame the loop gets going, not tolerance for a stalled one.
    let steady = SMOKE_FRAMES as u64 - first_go.expect("a GO strobe") as u64;
    assert!(
        m.vg_go_strobes >= steady / 2,
        "{} GO strobes over {steady} steady-state frames — the loop is not \
         drawing every frame",
        m.vg_go_strobes
    );

    // The attract screen animates — `ASTRD2.MAC:729-733` recomputes
    // `FLASHCOL` from `FRAME` every pass — so a hash that never changes means
    // the machine is wedged even if nothing crashed.
    let mut distinct = hashes.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(
        distinct.len() > 1,
        "the picture never changed across {SMOKE_FRAMES} frames"
    );
}
