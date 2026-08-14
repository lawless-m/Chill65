//! The two Tempest ROM builders must agree, byte for byte.
//!
//! The counterpart of `roms_sd.rs`, and it exists because
//! `chill65_asm::te::images` is **transcribed** from the builder in
//! `tests/common` rather than shared with it. That is deliberate — each
//! answers to its own needs, and neither can be bent out of shape by the
//! other — but transcriptions drift. This is the alarm.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/tempest \
//!   cargo test -p chill65-runtime --test roms_te -- --ignored --nocapture
//! ```

mod common;

use std::path::Path;

use chill65_runtime::cpu::Cpu;
use chill65_runtime::te::{run_frame_te, TeMachine, IRQS_PER_FRAME};

use common::{build_te_program, corpus, TE_PROG_BASE, TE_PROG_LEN, TE_VEC_ROM_BASE,
             TE_VEC_ROM_LEN};

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
fn both_builders_produce_the_same_roms() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let lib = chill65_asm::te::images(Path::new(&c)).expect("library build");
    let test = build_te_program(&c);
    let test_vec_rom = test.bytes(TE_VEC_ROM_BASE, TE_VEC_ROM_LEN);
    let test_prog = test.bytes(TE_PROG_BASE, TE_PROG_LEN);

    assert_eq!(lib.vec_rom.len(), TE_VEC_ROM_LEN, "vector ROM size");
    assert_eq!(lib.program.len(), TE_PROG_LEN, "program size");

    let vec_diffs = lib
        .vec_rom
        .iter()
        .zip(&test_vec_rom)
        .filter(|(a, b)| a != b)
        .count();
    let prog_diffs = lib
        .program
        .iter()
        .zip(&test_prog)
        .filter(|(a, b)| a != b)
        .count();

    println!(
        "vector ROM {} of {} bytes match\nprogram    {} of {} bytes match",
        TE_VEC_ROM_LEN - vec_diffs,
        TE_VEC_ROM_LEN,
        TE_PROG_LEN - prog_diffs,
        TE_PROG_LEN,
    );

    assert_eq!(vec_diffs, 0, "the two builders disagree on the vector ROM");
    assert_eq!(prog_diffs, 0, "the two builders disagree on the program");

    // Bytes two builders agree on are worth nothing if no machine runs them.
    let mut m = TeMachine::new();
    m.load_roms(&lib.vec_rom, &lib.program)
        .expect("the library images load");
    let mut cpu = Cpu::new();
    cpu.reset(&mut m);
    let stats = run_frame_te(&mut cpu, &mut m).expect("one frame");
    assert_eq!(stats.irqs_raised, IRQS_PER_FRAME, "the frame ran");
}
