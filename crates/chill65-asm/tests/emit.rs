//! The bucket-1 emitter, against original fixtures.
//!
//! No corpus needed. `tests/fixtures/hll.MAC` defines constructs of the same
//! names HLL65F uses and nothing else; nothing here is copied from the game.
//!
//! What these pin down is mostly **what the emitter refuses**. A lowering that
//! quietly mis-emits a routine it did not understand would be found much later,
//! as a divergence in a differential run, and be expensive to trace back. A
//! refusal is found immediately and costs only performance — plan §5's graceful
//! degradation, and the reason the creep line is safe.

use std::collections::HashMap;
use std::path::PathBuf;

use chill65_asm::assemble::{Assembler, SourceProvider};
use chill65_asm::emit::{emit, Emitted};
use chill65_asm::ir::Ir;

struct Fixtures {
    files: HashMap<String, String>,
    dir: PathBuf,
}

impl SourceProvider for Fixtures {
    fn load(&self, name: &str) -> Option<Vec<u8>> {
        if let Some(text) = self.files.get(name) {
            return Some(text.clone().into_bytes());
        }
        for cand in [name.to_string(), format!("{name}.MAC")] {
            if let Ok(bytes) = std::fs::read(self.dir.join(&cand)) {
                return Some(bytes);
            }
        }
        None
    }
}

fn build(main: &str) -> Ir {
    let p = Fixtures {
        files: HashMap::from([("MAIN.MAC".to_string(), main.to_string())]),
        dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
    };
    let mut a = Assembler::new(&p);
    a.assemble("MAIN.MAC").expect("assembly failed");
    a.ir.clone()
}

fn lower(main: &str, want: &[&str]) -> Emitted {
    emit(&build(main), want)
}

#[test]
fn a_straight_line_routine_lowers() {
    let e = lower(
        "	.=0A000\n\
         MAIN::	LDA I,012\n\
         	STA 0200\n\
         	TAX\n\
         	INX\n\
         	RTS\n",
        &["MAIN"],
    );
    assert!(e.refused.is_empty(), "refused: {:?}", e.refused);
    assert_eq!(e.functions.len(), 1);
    assert_eq!(e.functions[0].name, "MAIN");
    assert_eq!(e.functions[0].entry, 0xA000);
    assert_eq!(e.functions[0].instructions, 5);

    // The contract from `chill65-runtime`'s `Compiled`: a yield check before
    // each instruction, attribution, and cycles ticked from the helper.
    assert!(e.source.contains("pub fn r_MAIN"), "{}", e.source);
    assert!(e.source.contains("machine.begin_instruction(0xA000);"));
    assert!(e.source.contains("cpu.load_a(machine, Mode::Immediate, 2)"));
    assert!(e.source.contains("cpu.tax()"));
    assert!(e.source.contains("machine.irq_pending && !cpu.interrupt_disable"));
    assert!(e.source.contains("machine.tick(c);"));
    // Control transfer out is a yield.
    assert!(e.source.contains("cpu.rts(machine)"));
    // And the registry the dispatch reads.
    assert!(e.source.contains("(0xA000, r_MAIN),"), "{}", e.source);
}

#[test]
fn a_structured_conditional_becomes_a_rust_if() {
    let e = lower(
        "	.INCLUDE hll\n\
         	.=0A000\n\
         MAIN::	LDA I,01\n\
         	IFEQ\n\
         	INX\n\
         	ENDIF\n\
         	RTS\n",
        &["MAIN"],
    );
    assert!(e.refused.is_empty(), "refused: {:?}", e.refused);
    assert_eq!(e.functions.len(), 1);
    assert!(e.source.contains("if cpu.zero {"), "{}", e.source);
    // The construct's own branch is charged but does not jump: taken when the
    // block is skipped, which is the inverse of the tested condition.
    assert!(
        e.source.contains("machine.tick(if cpu.zero { 2 } else {"),
        "{}",
        e.source
    );
}

#[test]
fn a_structured_loop_becomes_a_rust_loop() {
    let e = lower(
        "	.INCLUDE hll\n\
         	.=0A000\n\
         MAIN::	BEGIN\n\
         	DEX\n\
         	EQEND\n\
         	RTS\n",
        &["MAIN"],
    );
    assert!(e.refused.is_empty(), "refused: {:?}", e.refused);
    assert!(e.source.contains("loop {"), "{}", e.source);
    // The construct names the condition that ENDS the loop, and emits the
    // inverse branch back to the top (dialect.md: `BEGIN … PLEND` assembles to
    // `BMI -3`). So the lowering breaks when the condition holds.
    assert!(e.source.contains("if cpu.zero { break; }"), "{}", e.source);
}

#[test]
fn a_hand_written_branch_becomes_a_state_machine() {
    let e = lower(
        "	.=0A000\n\
         MAIN::	LDX I,05\n\
         LOOP:	DEX\n\
         	BNE LOOP\n\
         	RTS\n",
        &["MAIN"],
    );
    assert!(e.refused.is_empty(), "refused: {:?}", e.refused);
    assert_eq!(e.functions.len(), 1);
    assert_eq!(
        e.functions[0].bucket,
        chill65_asm::emit::Bucket::StateMachine,
        "a bare branch is bucket 2, not bucket 1"
    );

    // Blocks are keyed by address, and the backward branch moves between them
    // rather than jumping.
    assert!(e.source.contains("let mut block: u16 = 0xA000;"), "{}", e.source);
    assert!(e.source.contains("match block {"), "{}", e.source);
    assert!(e.source.contains("block = 0xA002;"), "{}", e.source);
    // The branch's own condition, uninverted: BNE branches when Z is clear.
    assert!(e.source.contains("if !cpu.zero {"), "{}", e.source);
    // And anywhere with no block of its own yields to the interpreter.
    assert!(
        e.source.contains("_ => { cpu.pc = block; return done; }"),
        "{}",
        e.source
    );
}

#[test]
fn a_branch_out_of_the_routine_yields() {
    let e = lower(
        "	.=0A000\n\
         MAIN::	LDX I,05\n\
         	BNE AWAY\n\
         	RTS\n\
         AWAY::	RTS\n",
        &["MAIN"],
    );
    assert!(e.refused.is_empty(), "refused: {:?}", e.refused);
    // AWAY is its own routine, so the branch leaves: set pc and hand back.
    assert!(
        e.source.contains("if !cpu.zero { cpu.pc = 0xA005; return done; }"),
        "{}",
        e.source
    );
}

#[test]
fn an_unknown_routine_is_refused_rather_than_ignored() {
    let e = lower(
        "	.=0A000\n\
         MAIN::	RTS\n",
        &["NOSUCH"],
    );
    assert!(e.functions.is_empty());
    assert_eq!(e.refused.len(), 1);
    assert_eq!(e.refused[0].routine, "NOSUCH");
    assert!(e.refused[0].reason.contains("no such routine"));
}

#[test]
fn structured_and_state_machine_routines_coexist() {
    let e = lower(
        "	.=0A000\n\
         GOOD::	LDA I,01\n\
         	RTS\n\
         BAD::	LDX I,05\n\
         AGAIN:	DEX\n\
         	BNE AGAIN\n\
         	RTS\n",
        &["GOOD", "BAD"],
    );
    // Both lower now, by different routes: GOOD is straight-line, BAD has a
    // bare branch and so becomes a state machine.
    assert!(e.refused.is_empty(), "refused: {:?}", e.refused);
    assert_eq!(e.functions.len(), 2);
    let by = |n: &str| e.functions.iter().find(|f| f.name == n).expect(n);
    assert_eq!(by("GOOD").bucket, chill65_asm::emit::Bucket::Structured);
    assert_eq!(by("BAD").bucket, chill65_asm::emit::Bucket::StateMachine);
    assert!(e.source.contains("r_GOOD"));
    assert!(e.source.contains("r_BAD"));
}

#[test]
fn metadata_records_the_bucket_and_instruction_count() {
    let e = lower(
        "	.=0A000\n\
         MAIN::	LDA I,01\n\
         	RTS\n",
        &["MAIN"],
    );
    assert!(
        e.source.contains(r#"("MAIN", 0xA000, 2, "Structured")"#),
        "{}",
        e.source
    );
}
