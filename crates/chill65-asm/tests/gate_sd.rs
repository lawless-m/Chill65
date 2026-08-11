//! Space Duel's Phase 1 gate: reassemble the source and compare against the
//! original toolchain's own output.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/space-duel cargo test -p chill65-asm --test gate_sd -- --ignored --nocapture
//! ```
//!
//! The counterpart of `gate.rs`, which does the same for Crystal Castles. The
//! helpers below are copied from it rather than shared: the two gates guard
//! different games and must be able to move independently, and a shared
//! fixture that one game's needs bent out of shape would weaken the other.
//! `gate.rs` is not modified by this file's existence.
//!
//! # Which oracle, and why
//!
//! `SDGEN1.COM` — the original RT-11 build script, which survives in the source
//! tree — records exactly how the images were produced:
//!
//! ```text
//! VX0:AST2RD/L,VX0:A2LINK=VX0:AS2ROM,AST2RD,AST2RT,AS2SAC,AS2POK,AS2COI/C
//! VX0:A2NAME,AS2MSG,AS2FIL,AS2TST,A2IRQ,A2EARO/C
//! VX0:XYSIG,VGUTR2,A2GOOF
//! VX0:A2SHIP/L=VX0:A2SHIP
//! ```
//!
//! So `AST2RD.LDA` is linked from fifteen modules, and `A2SHIP.LDA` from one.
//! **This file starts with the single-module link.** `A2SHIP.MAC` is
//! `.ASECT`-only, so no linker is involved and it exercises the whole
//! comparison — assembler, oracle parse, address window — before the section
//! and placement work begins. `inventory.md` §7 has the full survey.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chill65_asm::assemble::{Assembler, SourceProvider};

struct Search {
    dirs: Vec<PathBuf>,
}

impl SourceProvider for Search {
    fn load(&self, name: &str) -> Option<Vec<u8>> {
        for d in &self.dirs {
            for cand in [name.to_string(), format!("{name}.MAC")] {
                if let Ok(b) = std::fs::read(d.join(&cand)) {
                    return Some(b);
                }
            }
        }
        None
    }
}

/// Parse a DEC absolute-loader image into an address-keyed map.
///
/// Records are `01 00 <count:16le> <addr:16le> <data…> <checksum>`; `count`
/// spans the six-byte header and excludes the checksum. Later records overwrite
/// earlier ones — HLL65F's `FND` rewinds the location counter to patch branch
/// operands, so the resolved image is what was burned, not the record stream.
fn load_lda(path: &PathBuf) -> BTreeMap<u16, u8> {
    let blob = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut mem = BTreeMap::new();
    let mut off = 0usize;
    while off + 6 <= blob.len() && blob[off] == 0x01 {
        let count = blob[off + 2] as usize | ((blob[off + 3] as usize) << 8);
        let addr = blob[off + 4] as u16 | ((blob[off + 5] as u16) << 8);
        if count < 6 || off + count >= blob.len() {
            break;
        }
        for (i, b) in blob[off + 6..off + count].iter().enumerate() {
            mem.insert(addr.wrapping_add(i as u16), *b);
        }
        off += count + 1;
    }
    mem
}

struct Built {
    image: BTreeMap<u16, u8>,
    symbols: Vec<(u16, String, String)>,
    errors: Vec<String>,
}

/// Assemble, and hand back the image **whether or not the assembly was clean**.
///
/// The fifteen-module program does not yet assemble without errors, and a
/// panic there would say only that. What the image contains at that point is
/// evidence: which modules landed, where, and against what. `Assembler` keeps
/// `image` populated on the error path, so the comparison can still run and
/// still report. Callers that require a clean assembly assert on `errors`
/// themselves — the ship gate does.
fn build(corpus: &str, roots: &[&str], dirs: &[&str]) -> Built {
    let root = PathBuf::from(corpus);
    let p = Search {
        dirs: dirs.iter().map(|d| root.join(d)).collect(),
    };
    let mut a = Assembler::new(&p);
    let (image, errors) = match a.assemble_units(roots) {
        Ok(img) => (img, Vec::new()),
        Err(errs) => (std::mem::take(&mut a.image), errs),
    };
    let mut symbols: Vec<(u16, String, String)> = a
        .globals
        .iter()
        .map(|(k, v)| (*v, k.clone(), "global".to_string()))
        .collect();
    for (unit, table) in &a.unit_locals {
        for (k, v) in table {
            if !k.contains('~') {
                symbols.push((*v, k.clone(), unit.clone()));
            }
        }
    }
    symbols.sort();
    Built {
        image,
        symbols,
        errors,
    }
}

/// Group the assembly errors by kind, so a report shows the shape of what is
/// left rather than seven hundred lines of it.
fn report_errors(errors: &[String]) {
    if errors.is_empty() {
        return;
    }
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut undefined: BTreeMap<&str, usize> = BTreeMap::new();
    for e in errors {
        let kind = if let Some(at) = e.find("undefined symbol ") {
            let name = e[at + "undefined symbol ".len()..]
                .split_whitespace()
                .next()
                .unwrap_or("?");
            *undefined.entry(name).or_default() += 1;
            "undefined symbol"
        } else if e.contains("requires an operand") {
            "cascade: requires an operand"
        } else if e.contains("branch out of range") {
            "cascade: branch out of range"
        } else {
            "other"
        };
        *kinds.entry(kind).or_default() += 1;
    }
    eprintln!("  {} assembly errors:", errors.len());
    let mut by_kind: Vec<_> = kinds.into_iter().collect();
    by_kind.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (k, n) in by_kind {
        eprintln!("    {n:6}  {k}");
    }
    let mut by_name: Vec<_> = undefined.into_iter().collect();
    by_name.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    eprintln!("  {} distinct undefined symbols; first ten:", by_name.len());
    for (k, n) in by_name.iter().take(10) {
        eprintln!("    {n:6}  {k}");
    }
    for e in errors.iter().filter(|e| {
        !e.contains("undefined symbol")
            && !e.contains("requires an operand")
            && !e.contains("branch out of range")
    }) {
        eprintln!("    other: {e}");
    }
}

fn nearest_symbol(syms: &[(u16, String, String)], addr: u16) -> String {
    match syms.binary_search_by_key(&addr, |(a, _, _)| *a) {
        Ok(i) => format!("{} [{}]", syms[i].1, syms[i].2),
        Err(0) => "<before first symbol>".into(),
        Err(i) => {
            let (a, n, u) = &syms[i - 1];
            format!("{n}+{:#x} [{u}]", addr - a)
        }
    }
}

/// Compare and, on mismatch, say exactly where and in what company.
fn compare(what: &str, built: &Built, oracle: &BTreeMap<u16, u8>, lo: u16, hi: u16) -> usize {
    let ours: Vec<u8> = (lo..=hi)
        .map(|a| *built.image.get(&a).unwrap_or(&0))
        .collect();
    let theirs: Vec<u8> = (lo..=hi).map(|a| *oracle.get(&a).unwrap_or(&0)).collect();

    let diffs: Vec<usize> = ours
        .iter()
        .zip(&theirs)
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();

    let total = ours.len();
    eprintln!(
        "{what}: {} of {total} bytes match ({:.2}%), {} differ",
        total - diffs.len(),
        100.0 * (total - diffs.len()) as f64 / total as f64,
        diffs.len()
    );

    if let Some(&first) = diffs.first() {
        let addr = lo + first as u16;
        let from = first.saturating_sub(8);
        let to = (first + 12).min(total);
        eprintln!("  first difference at {addr:04X}: ours {:02X}, oracle {:02X}",
            ours[first], theirs[first]);
        eprintln!("  nearest symbol : {}", nearest_symbol(&built.symbols, addr));
        eprintln!("  ours   {:04X}: {}", lo as usize + from,
            ours[from..to].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
        eprintln!("  oracle {:04X}: {}", lo as usize + from,
            theirs[from..to].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));

        // Contiguous runs, so a report shows structure rather than a list.
        let mut runs = Vec::new();
        let (mut s, mut p) = (diffs[0], diffs[0]);
        for &i in &diffs[1..] {
            if i == p + 1 {
                p = i;
            } else {
                runs.push((s, p));
                s = i;
                p = i;
            }
        }
        runs.push((s, p));
        eprintln!("  {} differing runs; first five:", runs.len());
        for (a, b) in runs.iter().take(5) {
            eprintln!("    {:04X}-{:04X} ({} bytes)  {}",
                lo as usize + a, lo as usize + b, b - a + 1,
                nearest_symbol(&built.symbols, lo + *a as u16));
        }
    }
    diffs.len()
}

fn corpus() -> Option<String> {
    std::env::var("CHILL65_CORPUS").ok()
}

/// The ship pictures, `2800-2FFF`, from the standalone single-module link.
///
/// No linking is involved: `A2SHIP.MAC` is `.ASECT`-only, so what this compares
/// is the assembler alone against the original's own output.
#[test]
#[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
fn ship_image_is_byte_identical() {
    let Some(c) = corpus() else {
        return;
    };
    let built = build(&c, &["A2SHIP.MAC"], &["."]);
    assert!(
        built.errors.is_empty(),
        "the single-module link must assemble cleanly:\n{:#?}",
        built.errors
    );
    let oracle = load_lda(&PathBuf::from(&c).join("A2SHIP.LDA"));
    let n = compare("SHIP", &built, &oracle, 0x2800, 0x2FFF);
    assert_eq!(n, 0, "ship image must match the oracle exactly");
}

/// The fifteen-module program, `0000-8FFF`, against `ASTRD2.LDA`.
///
/// `SDGEN1.COM`'s link order, with `ASTRD2` in place of `AST2RD`: the two are
/// revisions of one module, `ASTRD2` is the later release and pairs with the
/// `ASTRD2.LDA` oracle, and it assembles at 413 errors alone against
/// `AST2RD.MAC`'s 3,401. `inventory.md` §7 items 3b and 3f record how that was
/// established, including a retraction of the first reasoning for it.
///
/// **This does not pass yet, and it is committed failing on purpose.** The
/// assembly still reports errors and no linker exists, so what this gate does
/// today is measure: how much of the image is right, where the first
/// difference is, what symbol it sits in, and how the differences are shaped.
/// The assertion is the target and stays as written — weakening it to make the
/// suite green would leave nothing that could tell us when the target is met.
/// It is `#[ignore]`d, so a default `cargo test` is unaffected.
#[test]
#[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
fn program_image_is_byte_identical() {
    let Some(c) = corpus() else {
        return;
    };
    const ROOTS: [&str; 15] = [
        "AS2ROM.MAC",
        "ASTRD2.MAC",
        "AST2RT.MAC",
        "AS2SAC.MAC",
        "AS2POK.MAC",
        "AS2COI.MAC",
        "A2NAME.MAC",
        "AS2MSG.MAC",
        "AS2FIL.MAC",
        "AS2TST.MAC",
        "A2IRQ.MAC",
        "A2EARO.MAC",
        "XYSIG.MAC",
        "VGUTR2.MAC",
        "A2GOOF.MAC",
    ];
    let built = build(&c, &ROOTS, &["."]);
    report_errors(&built.errors);
    let oracle = load_lda(&PathBuf::from(&c).join("ASTRD2.LDA"));
    let n = compare("PROGRAM", &built, &oracle, 0x0000, 0x8FFF);
    assert_eq!(n, 0, "program image must match the oracle exactly");
}
