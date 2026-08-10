//! Structure markers and routine segmentation, against an original fixture.
//!
//! No corpus needed: `tests/fixtures/hll.MAC` defines constructs of the same
//! names as HLL65F's and nothing else. Nothing here is copied from the game
//! source — see the fixture's own header.

use std::collections::HashMap;
use std::path::PathBuf;

use chill65_asm::assemble::{Assembler, SourceProvider};
use chill65_asm::ir::{Ir, Marker};

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
fn constructs_are_classified_by_name() {
    assert_eq!(
        Marker::classify("IFEQ"),
        Some(Marker::IfOpen { cond: "EQ".into() })
    );
    assert_eq!(
        Marker::classify("ifcs"),
        Some(Marker::IfOpen { cond: "CS".into() }),
        "names are case-insensitive, as the dialect is"
    );
    assert_eq!(Marker::classify("ELSE"), Some(Marker::Else));
    assert_eq!(Marker::classify("ENDIF"), Some(Marker::IfClose));
    assert_eq!(Marker::classify("THEN"), Some(Marker::IfClose));
    assert_eq!(Marker::classify("BEGIN"), Some(Marker::LoopOpen));
    assert_eq!(
        Marker::classify("PLEND"),
        Some(Marker::LoopClose { cond: "PL".into() })
    );
    assert_eq!(
        Marker::classify("VCCONT"),
        Some(Marker::LoopContinue { cond: "VC".into() })
    );
    assert_eq!(Marker::classify("LDA"), None, "not every macro is a construct");
    assert_eq!(Marker::classify("TRAI"), None);
}

#[test]
fn markers_are_recorded_where_the_construct_is_invoked() {
    let ir = build(
        "	.INCLUDE hll\n\
         	.=0A000\n\
         START:	LDA I,01\n\
         	IFEQ\n\
         	NOP\n\
         	ENDIF\n\
         	RTS\n",
    );
    let markers: Vec<_> = ir.markers().map(|(m, _)| m.clone()).collect();
    assert_eq!(
        markers,
        vec![Marker::IfOpen { cond: "EQ".into() }, Marker::IfClose]
    );
}

#[test]
fn markers_balance_and_interleave_with_the_instructions() {
    let ir = build(
        "	.INCLUDE hll\n\
         	.=0A000\n\
         START:	BEGIN\n\
         	LDA I,01\n\
         	IFCS\n\
         	NOP\n\
         	ELSE\n\
         	NOP\n\
         	ENDIF\n\
         	EQEND\n\
         	RTS\n",
    );
    let markers: Vec<_> = ir.markers().map(|(m, _)| m.clone()).collect();
    assert_eq!(
        markers,
        vec![
            Marker::LoopOpen,
            Marker::IfOpen { cond: "CS".into() },
            Marker::Else,
            Marker::IfClose,
            Marker::LoopClose { cond: "EQ".into() },
        ]
    );

    let mut depth = 0i32;
    for m in &markers {
        if m.opens() {
            depth += 1;
        }
        if m.closes() {
            depth -= 1;
        }
        assert!(depth >= 0, "a construct closed before it opened");
    }
    assert_eq!(depth, 0, "constructs do not balance");
}

#[test]
fn routines_start_at_called_or_global_labels() {
    let ir = build(
        "	.INCLUDE hll\n\
         	.=0A000\n\
         MAIN::	JSR HELPER\n\
         	RTS\n\
         NOTCALLED:	NOP\n\
         HELPER:	LDA I,02\n\
         	RTS\n",
    );
    // MAIN is global, HELPER is a JSR target. NOTCALLED is neither, so it
    // belongs to whatever routine it falls inside rather than starting one.
    assert_eq!(ir.routines(), vec!["MAIN".to_string(), "HELPER".to_string()]);

    let by_addr = |a: u16| {
        ir.instructions()
            .find(|i| i.addr == a)
            .and_then(|i| i.routine.clone())
    };
    assert_eq!(by_addr(0xA000).as_deref(), Some("MAIN"), "the JSR");
    assert_eq!(by_addr(0xA003).as_deref(), Some("MAIN"), "its RTS");
    assert_eq!(
        by_addr(0xA004).as_deref(),
        Some("MAIN"),
        "NOTCALLED does not start a routine, so its NOP stays in MAIN"
    );
    assert_eq!(by_addr(0xA005).as_deref(), Some("HELPER"));
}

#[test]
fn instructions_before_any_routine_are_left_unassigned() {
    let ir = build(
        "	.=0A000\n\
         	NOP\n\
         CALLED:	RTS\n\
         	JSR CALLED\n",
    );
    let first = ir.instructions().next().expect("an instruction");
    assert_eq!(first.addr, 0xA000);
    assert_eq!(
        first.routine, None,
        "nothing reaches this NOP, so it belongs to no routine"
    );
}
