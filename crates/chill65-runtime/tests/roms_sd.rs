//! The library ROM builder agrees with the test one, byte for byte.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/space-duel \
//!   cargo test -p chill65-runtime --test roms_sd -- --ignored --nocapture
//! ```
//!
//! # Why this test exists
//!
//! `chill65_asm::sd::images` is transcribed from `tests/common/mod.rs` rather
//! than shared with it, so that the gate's builder and the runnable one can
//! each answer to their own needs. Copies drift. This is the check that they
//! have not: it builds both ways and compares every byte of both images, so a
//! divergence fails here rather than showing up as a game that misbehaves for
//! no visible reason.
//!
//! It also runs a frame, because agreeing on bytes that no machine will accept
//! would prove very little.

mod common;

use chill65_runtime::sd::{run_frame_sd, SdMachine};
use chill65_runtime::Cpu;

use common::{build_sd_program, build_sd_ship, corpus, SD_PROG_LEN};

#[test]
#[ignore = "needs CHILL65_CORPUS"]
fn the_library_builder_matches_the_test_builder() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let lib = chill65_asm::sd::images(&c).expect("library build");
    let ship = build_sd_ship(&c);
    let program = build_sd_program(&c).bytes(0x0000, SD_PROG_LEN);

    assert_eq!(lib.ship.len(), ship.len(), "ship image size");
    assert_eq!(lib.program.len(), program.len(), "program image size");

    let ship_diff = lib.ship.iter().zip(&ship).filter(|(a, b)| a != b).count();
    let prog_diff = lib
        .program
        .iter()
        .zip(&program)
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "\nship:    {} of {} bytes match, {ship_diff} differ",
        ship.len() - ship_diff,
        ship.len()
    );
    println!(
        "program: {} of {} bytes match, {prog_diff} differ",
        program.len() - prog_diff,
        program.len()
    );
    assert_eq!(ship_diff, 0, "the two ship builds disagree");
    assert_eq!(prog_diff, 0, "the two program builds disagree");

    // Bytes two builders agree on are still worth nothing if no machine will
    // run them.
    let mut m = SdMachine::new();
    m.load_roms(&lib.ship, &lib.program).expect("load");
    m.set_options(0, 0);
    m.self_test = false;
    let mut cpu = Cpu::new();
    cpu.reset(&mut m);
    let stats = run_frame_sd(&mut cpu, &mut m).expect("one frame");
    assert_eq!(stats.irqs_raised, 4, "a frame raises four interrupts");
    println!("one frame ran from the library images: {stats:?}\n");
}
