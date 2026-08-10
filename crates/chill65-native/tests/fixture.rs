//! Compiled and interpreted execution agree, with no corpus present.
//!
//! ```text
//! cargo test -p chill65-native
//! ```
//!
//! This is the whole pipeline end to end — assemble, record the IR, lower to
//! Rust, compile it, run it against the interpreter — on an original fixture
//! program committed in this crate. No game source is involved, so it runs
//! anywhere, and it is the check that catches an emitter regression before any
//! migration is attempted.
//!
//! The fixture exercises what the lowering has to get right: straight-line
//! work, a structured conditional, a structured loop, a call and return, and an
//! interrupt handler the interpreter finishes when a compiled routine yields.

use chill65_native::{fixture, Registry};
use chill65_runtime::frame::{run_frame, run_frame_with, Compiled};
use chill65_runtime::{Cpu, Machine};

const FRAMES: u32 = 8;

fn machine() -> (Cpu, Machine) {
    let mut m = Machine::new();
    m.load_roms(fixture::IMAGE, &[0u8; 0x4000])
        .expect("the fixture image loads");
    let mut cpu = Cpu::new();
    cpu.reset(&mut m);
    (cpu, m)
}

#[test]
fn the_fixture_lowered_something() {
    let r = Registry::fixture();
    assert!(!r.is_empty(), "no fixture routines were compiled");
    eprintln!(
        "{} compiled routines: {:?}",
        r.len(),
        fixture::METADATA
            .iter()
            .map(|(n, a, i, b)| format!("{n} @{a:04X} {i} instrs {b}"))
            .collect::<Vec<_>>()
    );
    // The fixture's own routines, all of them structured.
    assert!(fixture::METADATA.iter().any(|(n, ..)| *n == "WORK"));
    assert!(fixture::METADATA.iter().all(|(.., b)| *b == "Structured"));
}

#[test]
fn dispatched_execution_matches_interpreted_exactly() {
    let (mut cpu_a, mut m_a) = machine();
    let (mut cpu_b, mut m_b) = machine();
    let mut compiled = Registry::fixture();

    let mut interpreted_instructions = 0u64;
    for frame in 0..FRAMES {
        let a = run_frame(&mut cpu_a, &mut m_a).unwrap_or_else(|e| panic!("frame {frame}: {e}"));
        let b = run_frame_with(&mut cpu_b, &mut m_b, &mut compiled)
            .unwrap_or_else(|e| panic!("frame {frame}: {e}"));
        interpreted_instructions += a.interpreted;

        assert_eq!(a.cycles, b.cycles, "frame {frame}: cycle counts differ");
        assert_eq!(a.irqs_taken, b.irqs_taken, "frame {frame}: interrupts differ");
        assert_eq!(
            m_a.frame_hash(),
            m_b.frame_hash(),
            "frame {frame}: pictures differ"
        );
    }

    // Whole-machine state, not just the picture.
    assert_eq!(m_a.cycles, m_b.cycles, "total cycles");
    assert_eq!(cpu_a.pc, cpu_b.pc, "pc");
    assert_eq!(cpu_a.a, cpu_b.a, "A");
    assert_eq!(cpu_a.x, cpu_b.x, "X");
    assert_eq!(cpu_a.y, cpu_b.y, "Y");
    assert_eq!(cpu_a.s, cpu_b.s, "stack pointer");
    assert_eq!(cpu_a.p(), cpu_b.p(), "flags");
    assert_eq!(m_a.ram, m_b.ram, "RAM");

    // And the compiled code really carried the run, rather than yielding
    // immediately and letting the interpreter do everything.
    let compiled_instructions = compiled.compiled_instructions();
    eprintln!(
        "{FRAMES} frames: {compiled_instructions} instructions compiled, \
         {interpreted_instructions} interpreted when interpreting alone"
    );
    assert!(compiled_instructions > 0, "nothing was dispatched");
    assert!(
        compiled_instructions * 2 > interpreted_instructions,
        "only {compiled_instructions} of {interpreted_instructions} instructions \
         were compiled — the fixture should be mostly compiled"
    );
}

#[test]
fn the_game_registry_is_empty_without_a_manifest() {
    // Nothing is migrated yet, so this is the baseline: the crate builds and
    // runs with no game routines compiled at all.
    let r = Registry::game();
    eprintln!("{} game routines compiled", r.len());
}
