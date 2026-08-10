//! Attract mode: it runs, it draws, and it does the same thing twice.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-runtime --test attract -- --ignored --nocapture
//! ```
//!
//! # Why determinism is the interesting assertion
//!
//! It looks like a formality and is not. The game reads POKEY's `RANDOM` to
//! make **gameplay** decisions — `CATOUT.MAC:22` picks the cat's X position
//! from it — so an identical frame hash across two independent runs is evidence
//! that the LFSR is clocked from machine cycles and nothing host-dependent has
//! leaked in. A non-deterministic interpreter would make Phase 3's differential
//! harness against MAME impossible to write.
//!
//! The watchdog assertion matters for a similar reason: if the game crashes,
//! the board resets itself and carries on, so a crashed run still *looks* alive.
//! An expired watchdog is the only signal that happened.

mod common;

use chill65_runtime::frame::run_frame;
use chill65_runtime::{Cpu, Machine};

/// Long enough to be well past the power-on self-test (which ends around frame
/// 353 — see `hardware.md` §10.2) and into attract mode proper.
const FRAMES: u32 = 600;

struct Outcome {
    hash: u64,
    lit: usize,
    cycles: u64,
    watchdog_expired: bool,
    watchdog_strobes: u64,
    bank_switches: u64,
}

fn run_attract(prog: &[u8], data: &[u8], mob: &[u8], frames: u32) -> Outcome {
    let mut m = Machine::new();
    m.load_roms(prog, data).expect("load");
    // Motion objects are part of the picture, so they are part of the hash.
    m.load_motion_roms(mob).expect("motion roms");
    let mut cpu = Cpu::new();
    cpu.reset(&mut m);

    for frame in 0..frames {
        run_frame(&mut cpu, &mut m).unwrap_or_else(|e| {
            panic!("{e} at frame {frame} ({} cycles), PC {:04X}", m.cycles, cpu.pc)
        });
    }

    let picture = m.framebuffer();
    Outcome {
        hash: m.frame_hash(),
        // Background is colour RAM entry 16 now, not index 0.
        lit: picture
            .iter()
            .filter(|&&p| p != chill65_runtime::video::BITMAP_CRAM_BASE as u8)
            .count(),
        cycles: m.cycles,
        watchdog_expired: m.watchdog_expired,
        watchdog_strobes: m.watchdog_strobes,
        bank_switches: m.bank_switches,
    }
}

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn attract_mode_runs_draws_and_is_deterministic() {
    let Some(corpus) = common::corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let (prog, data) = common::build_images(&corpus);
    let mob = common::motion_rom_image(&corpus);

    let first = run_attract(&prog, &data, &mob, FRAMES);
    eprintln!(
        "attract: {FRAMES} frames, {} cycles, hash {:016x}\n\
         \x20 {} lit pixels, {} watchdog strobes, {} bank switches",
        first.cycles, first.hash, first.lit, first.watchdog_strobes, first.bank_switches,
    );

    // No illegal opcode — `run_attract` panics on one, naming the frame and PC.

    assert!(
        !first.watchdog_expired,
        "the watchdog expired: the game crashed and the board would have reset \
         itself, which is otherwise a silent failure"
    );
    assert!(
        first.lit > 1000,
        "only {} lit pixels — attract mode is not drawing",
        first.lit
    );

    // The assertion this test exists for.
    let second = run_attract(&prog, &data, &mob, FRAMES);
    assert_eq!(
        first.hash, second.hash,
        "two independent runs produced different pictures ({:016x} vs {:016x}) — \
         something host-dependent has leaked into the machine, and Phase 3's \
         differential harness cannot be built on it",
        first.hash, second.hash
    );
    assert_eq!(first.cycles, second.cycles, "cycle counts diverged");
    assert_eq!(first.lit, second.lit);

    // A shorter run must differ, or the "identical" result above would be
    // vacuous — it would pass just as well on a machine that never changed.
    let shorter = run_attract(&prog, &data, &mob, FRAMES / 2);
    assert_ne!(
        first.hash, shorter.hash,
        "300 and 600 frames give the same picture, so the equality above proves \
         nothing about determinism"
    );
}
