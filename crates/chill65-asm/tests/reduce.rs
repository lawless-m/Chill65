//! The reducibility measurement, against original fixtures.
//!
//! No corpus needed. Two programs, both written here: one whose loop is entered
//! at one block, and one whose loop is entered at two. The second is the shape
//! that makes a graph irreducible, and the reason `codegen-readiness.md` said
//! the question mattered for WASM.

use std::collections::HashMap;
use std::path::PathBuf;

use chill65_asm::assemble::{Assembler, SourceProvider};
use chill65_asm::ir::Ir;
use chill65_asm::reduce::{measure, Verdict};

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

#[test]
fn a_backward_branch_loop_is_reducible() {
    // One loop, entered only at its head: the ordinary shape.
    let r = measure(&build(
        "	.=0A000\n\
         MAIN::	LDX I,05\n\
         LOOP:	DEX\n\
         	BNE LOOP\n\
         	RTS\n",
    ));
    assert_eq!(r.routines.len(), 1, "{:?}", r.routines);
    assert_eq!(r.routines[0].name, "MAIN");
    assert_eq!(r.routines[0].hand_branches, 1);
    assert_eq!(r.routines[0].retreating, 1, "the loop should be found");
    assert_eq!(r.routines[0].verdict, Verdict::Reducible);
    assert!(r.routines[0].offenders.is_empty());
    assert_eq!(r.branches_in(Verdict::Reducible), 1);
    assert_eq!(r.branches_in(Verdict::Irreducible), 0);
}

#[test]
fn a_branch_into_the_middle_of_a_loop_is_irreducible() {
    // The entry reaches both halves of the loop directly, so neither half
    // dominates the other and the retreating edge is not a back edge.
    //
    //   A000 --taken--> A005
    //     |              |  ^
    //     v              v  |
    //   A004 <-----------+  |
    //     +------------------+
    //
    // Which of the two edges between A004 and A005 the search calls the
    // retreating one depends on the order it walks them; the verdict does not.
    let r = measure(&build(
        "	.=0A000\n\
         MAIN::	LDX I,05\n\
         	BNE MID\n\
         TOP:	DEX\n\
         MID:	INX\n\
         	BNE TOP\n\
         	RTS\n",
    ));
    assert_eq!(r.routines.len(), 1, "{:?}", r.routines);
    assert_eq!(r.routines[0].hand_branches, 2);
    assert_eq!(r.routines[0].retreating, 1);
    assert_eq!(r.routines[0].verdict, Verdict::Irreducible);
    assert_eq!(r.routines[0].offenders.len(), 1);
    let (from, to) = r.routines[0].offenders[0];
    assert_eq!(
        [from.min(to), from.max(to)],
        [0xA004, 0xA005],
        "the two-entry loop is the offender"
    );
    assert_eq!(r.branches_in(Verdict::Irreducible), 2);
    assert_eq!(r.branches_in(Verdict::Reducible), 0);
}

#[test]
fn routines_without_a_hand_written_branch_are_not_classified() {
    // Straight-line code has nothing to say about reducibility, and the
    // measurement is of the hand-written flow specifically.
    let r = measure(&build(
        "	.=0A000\n\
         MAIN::	LDA I,012\n\
         	RTS\n",
    ));
    assert!(r.routines.is_empty(), "{:?}", r.routines);
    assert_eq!(r.hand_branches(), 0);
}

#[test]
fn a_structured_loop_is_not_counted_as_hand_written() {
    // `EQEND`'s branch comes from HLL65F, so it belongs to the 64.4% the
    // emitter already carries through as native Rust — not to this measurement.
    let r = measure(&build(
        "	.INCLUDE hll\n\
         	.=0A000\n\
         MAIN::	BEGIN\n\
         	DEX\n\
         	EQEND\n\
         	RTS\n",
    ));
    assert!(r.routines.is_empty(), "{:?}", r.routines);
}
