//! Tempest's own ROM diagnostic, run on the image our assembler built.
//!
//! The counterpart of `selftest_sd.rs`, and the strongest witness available:
//! the game checks the Phase 1 build with the original's own arithmetic, from
//! inside the game, and writes its verdict into RAM where it can be read out.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/tempest \
//!   cargo test -p chill65-runtime --test selftest_te -- --ignored --nocapture
//! ```
//!
//! # What each byte means
//!
//! `ALTEST.MAC:23-28` lays the result block over `CBUF1`: `MBCOND`, `RAMCND`,
//! `PK1CND`, `PK2CND`, `EARCND`, then twelve `CHKSMS` bytes. Every one is a
//! *fault counter* — zero is good, and each subsystem increments its own when
//! it disagrees.
//!
//! - **`CHKSMS`**, twelve bytes. `ALTEST.MAC:364-400`'s `ROMTST` walks
//!   `NROMS=12` blocks of 2K from `ROMSTART=3000`, seeding each with its own
//!   ROM index and EORing 2,048 bytes, so a good block sums back to its seed
//!   and stores zero. Blocks 0-1 are the vector ROM at `3000-3FFF`; the
//!   pointer then reloads to `9000` and blocks 2-11 cover the program ROM.
//!   The `CHKSM0`-`CHKSMB` bytes Atari planted through the source are what
//!   make each block come out right, so a non-zero byte here is the game
//!   saying our image is not the one it was built from.
//! - **`MBCOND`**, the math box. `ALTEST.MAC:759-799` divides a value by
//!   itself with `N=0x10` and requires the quotient to be exactly 1, within a
//!   100-iteration poll. It walks the operand upward every pass, so this is
//!   not one division but a different one each time.
//! - **`PK1CND`/`PK2CND`** require each POKEY's `RANDOM` to advance.
//! - **`EARCND`** requires the EAROM read path to clock data out and checksum
//!   cleanly (`ALEARO.MAC:215-249`).
//!
//! Addresses are resolved through the symbol table rather than written down,
//! and read with `peek` so that inspecting the machine cannot perturb it.

mod common;

use chill65_runtime::bus::Bus;
use chill65_runtime::cpu::Cpu;
use chill65_runtime::te::{run_frame_te, TeMachine};

use common::{build_te_program, corpus, TE_PROG_BASE, TE_PROG_LEN, TE_VEC_ROM_BASE,
             TE_VEC_ROM_LEN};

/// Frames to run before reading the verdict.
///
/// The power-on sequence is zero page, a RAM and vector-RAM page walk,
/// `ROMTST`, the POKEY checks and the EAROM read; the math box test then runs
/// continuously. This is far more than the sequence needs, so the continuous
/// tests get many passes and a counter that only trips occasionally still
/// shows.
const FRAMES: u32 = 400;

/// The twelve per-ROM checksum bytes.
const NROMS: usize = 12;

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
fn the_games_own_diagnostic_passes() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let program = build_te_program(&c);
    let mut m = TeMachine::new();
    m.load_roms(
        &program.bytes(TE_VEC_ROM_BASE, TE_VEC_ROM_LEN),
        &program.bytes(TE_PROG_BASE, TE_PROG_LEN),
    )
    .expect("load");

    // Hold the self-test switch. `ALTEST.MAC:280-282` reads `IN1 & MTEST` and
    // takes the *normal* boot when it is non-zero, so the switch is active
    // low and holding it means the bit reads clear.
    m.input.self_test = true;

    let mbcond = program.symbol("MBCOND").expect("MBCOND");
    let chksms = program.symbol("CHKSMS").expect("CHKSMS");
    println!("MBCOND {mbcond:04X}, CHKSMS {chksms:04X}");
    assert!(mbcond < 0x0800, "the result block lives in RAM");

    let mut cpu = Cpu::new();
    cpu.reset(&mut m);

    for frame in 0..FRAMES {
        run_frame_te(&mut cpu, &mut m)
            .unwrap_or_else(|e| panic!("CPU error on frame {frame}: {e:?}"));
        assert!(
            !m.watchdog_expired,
            "the watchdog expired on frame {frame} during the self-test"
        );
    }

    // Read the verdict without disturbing anything.
    let names = ["MBCOND", "RAMCND", "PK1CND", "PK2CND", "EARCND"];
    let flags: Vec<u8> = (0..names.len())
        .map(|i| m.peek(mbcond + i as u16))
        .collect();
    let sums: Vec<u8> = (0..NROMS).map(|i| m.peek(chksms + i as u16)).collect();

    println!(
        "after {FRAMES} frames: {} GO strobes, {} segments\n  \
         condition flags  {}\n  \
         ROM checksums    {}",
        m.vg_go_strobes,
        m.segments.len(),
        names
            .iter()
            .zip(&flags)
            .map(|(n, v)| format!("{n}={v}"))
            .collect::<Vec<_>>()
            .join(" "),
        sums.iter()
            .map(|v| format!("{v:02X}"))
            .collect::<Vec<_>>()
            .join(" "),
    );

    assert!(
        sums.iter().all(|v| *v == 0),
        "the game disagrees with our image: {sums:02X?}"
    );
    for (name, value) in names.iter().zip(&flags) {
        assert_eq!(*value, 0, "{name} is {value}, so the game found a fault");
    }

    assert!(m.vg_go_strobes > 0, "the test screen was never started");
    assert!(!m.segments.is_empty(), "the test screen drew nothing");
}
