//! The IR, checked against the real game build.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-asm --test ir -- --ignored --nocapture
//! ```
//!
//! Two assertions, and the second is what makes the first worth having.
//!
//! **It re-encodes to the image.** Every instruction the IR records, encoded
//! again from what was recorded, reproduces the bytes actually emitted at that
//! address. An IR that merely looks plausible would pass every unit test and
//! then quietly mislower a routine; this is the check that it *is* the program.
//!
//! **Its provenance matches the published measurement.** `codegen-readiness.md`
//! attributed all 876 branches by macro origin — 564 structured, 46 other
//! macros, 266 hand-written. The IR reaches the same numbers by carrying the
//! expansion chain per instruction rather than by counting as it goes, so the
//! two are independent routes to one figure.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use chill65_asm::assemble::{Assembler, SourceProvider};

/// The macro names HLL65F emits its branches from — the same list the
/// assembler's own `analyse()` uses.
const HLL: &[&str] = &[
    "IFXX", "FND", "ELSE", "THEN", "ENDIF", "ENDC", "BEGIN", "LOC", "..END", "CONTINUE", "IF",
];

fn corpus() -> Option<PathBuf> {
    std::env::var("CHILL65_CORPUS").ok().map(PathBuf::from)
}

struct Search {
    dirs: Vec<PathBuf>,
}

impl SourceProvider for Search {
    fn load(&self, name: &str) -> Option<Vec<u8>> {
        // `.MAC` is implicit on includes, as gate.rs's provider also allows.
        for dir in &self.dirs {
            for cand in [name.to_string(), format!("{name}.MAC")] {
                if let Ok(bytes) = std::fs::read(dir.join(&cand)) {
                    return Some(bytes);
                }
            }
        }
        None
    }
}

fn build_program(corpus: &PathBuf) -> (BTreeMap<u16, u8>, chill65_asm::ir::Ir) {
    let p = Search {
        dirs: vec![corpus.join("version-3"), corpus.clone()],
    };
    let mut a = Assembler::new(&p);
    let image = a
        .assemble_units(&["CRF.MAC", "CRP.MAC", "CLS.MAC"])
        .unwrap_or_else(|e| panic!("assembly failed:\n{e:#?}"));
    (image, a.ir.clone())
}

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn the_ir_re_encodes_to_the_game_image() {
    let Some(corpus) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let (image, ir) = build_program(&corpus);

    let instructions = ir.instructions().count();
    eprintln!(
        "IR: {} events, {instructions} instructions, {} labels, {} data runs",
        ir.events.len(),
        ir.labels().count(),
        ir.data().count()
    );
    assert!(instructions > 5000, "only {instructions} instructions recorded");

    let problems = ir.check_against(&image);
    assert!(
        problems.is_empty(),
        "{} of {instructions} instructions do not re-encode to the image:\n  {}",
        problems.len(),
        problems.iter().take(10).cloned().collect::<Vec<_>>().join("\n  ")
    );
}

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn ir_provenance_reproduces_the_published_branch_attribution() {
    let Some(corpus) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let (_, ir) = build_program(&corpus);

    let mut structured = 0;
    let mut other_macro = 0;
    let mut direct = 0;
    for i in ir.instructions().filter(|i| i.is_branch()) {
        if i.chain.is_empty() {
            direct += 1;
        } else if i.expanded_from(HLL) {
            structured += 1;
        } else {
            other_macro += 1;
        }
    }
    let total = structured + other_macro + direct;
    eprintln!(
        "branches: {structured} structured, {other_macro} other macro, \
         {direct} hand-written, {total} total ({:.1}% structured)",
        100.0 * structured as f64 / total as f64
    );

    // codegen-readiness.md, measured independently by the assembler counting as
    // it emitted. The IR reaches the same figures from recorded provenance.
    assert_eq!(structured, 564, "structured branches");
    assert_eq!(other_macro, 46, "other-macro branches");
    assert_eq!(direct, 266, "hand-written branches");
    assert_eq!(total, 876, "total branches");
}

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn the_ir_records_every_symbol_the_assembler_defined() {
    let Some(corpus) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let p = Search {
        dirs: vec![corpus.join("version-3"), corpus.clone()],
    };
    let mut a = Assembler::new(&p);
    a.assemble_units(&["CRF.MAC", "CRP.MAC", "CLS.MAC"])
        .expect("assembly failed");

    // Every address label in the IR must agree with the symbol table the
    // assembler resolved, or routine segmentation is building on sand.
    let mut checked = 0usize;
    let mut tables: HashMap<&str, &HashMap<String, u16>> = HashMap::new();
    for (unit, table) in &a.unit_locals {
        tables.insert(unit.as_str(), table);
    }
    for label in a.ir.labels() {
        let known = a
            .globals
            .get(&label.name.to_ascii_uppercase())
            .or_else(|| {
                tables
                    .get(label.unit.as_str())
                    .and_then(|t| t.get(&label.name.to_ascii_uppercase()))
            })
            .copied();
        if let Some(addr) = known {
            assert_eq!(
                addr, label.addr,
                "{} in {}: IR says {:04X}, symbol table says {addr:04X}",
                label.name, label.unit, label.addr
            );
            checked += 1;
        }
    }
    eprintln!("{checked} IR labels agree with the assembler's symbol table");
    assert!(checked > 500, "only {checked} labels cross-checked");
}
