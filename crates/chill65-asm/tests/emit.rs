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
fn a_hand_written_branch_is_refused_by_name() {
    let e = lower(
        "	.=0A000\n\
         MAIN::	LDX I,05\n\
         LOOP:	DEX\n\
         	BNE LOOP\n\
         	RTS\n",
        &["MAIN"],
    );
    assert!(e.functions.is_empty(), "should not have lowered anything");
    assert_eq!(e.refused.len(), 1);
    let r = &e.refused[0];
    assert_eq!(r.routine, "MAIN");
    assert!(
        r.reason.contains("hand-written branch") && r.reason.contains("relooper"),
        "unhelpful reason: {}",
        r.reason
    );
    // Nothing half-emitted was left behind.
    assert!(!e.source.contains("r_MAIN"), "{}", e.source);
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
fn refusing_one_routine_does_not_stop_the_others() {
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
    assert_eq!(e.functions.len(), 1, "GOOD should still lower");
    assert_eq!(e.functions[0].name, "GOOD");
    assert_eq!(e.refused.len(), 1);
    assert_eq!(e.refused[0].routine, "BAD");
    assert!(e.source.contains("r_GOOD"));
    assert!(!e.source.contains("r_BAD"));
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
