//! Tempest's Phase 1 gate: reassemble the source and compare against the
//! original toolchain's own output.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/tempest cargo test -p chill65-asm --test gate_te -- --ignored --nocapture
//! ```
//!
//! The counterpart of `gate.rs` and `gate_sd.rs`. The helpers below are copied
//! from `gate_sd.rs` rather than shared, for the reason that file gives: the
//! gates guard different games and must be able to move independently. Neither
//! of the other two is modified by this file's existence.
//!
//! # Which oracle, and why
//!
//! Tempest ships **two** linked images, and they are different builds. They
//! differ in 1,027 bytes across 53 runs, concentrated above `D7E1`, and 28
//! addresses exist in one and not the other. Two module sets exist to match:
//! the plain `ALSCOR`/`ALDISP`/`ALHARD`/`ALTEST` and the later
//! `ALSCO2`/`ALDIS2`/`ALHAR2`/`ALTES2`.
//!
//! **`TEMPST.LDA` is the plain-module build; `ALEXEC.LDA` is the starred one.**
//! The names mislead — `ALEXEC.MAP` documents the build that `TEMPST.LDA`
//! holds, not the one sharing its name.
//!
//! That is settled by the game's own bookkeeping rather than by inference.
//! `ALHARD.MAC` and `ALHAR2.MAC` each declare twelve `QCHKS` constants — the
//! bytes Atari planted so every 2K block EORs to its own ROM index — and they
//! disagree on exactly three. `ALEXEC.MAP` gives those three addresses, and
//! each image carries its own variant's values at them:
//!
//! ```text
//!   symbol  addr    ALHARD  ALHAR2  |  TEMPST.LDA  ALEXEC.LDA
//!   CHKSM5  A8AF        B2      E1  |          B2          E1
//!   CHKSM6  B497        1E      1D  |          1E          1D
//!   CHKSMB  DDDC        EE      73  |          EE          73
//! ```
//!
//! Six agreements, no exceptions, in both directions — and the finished link
//! settles it beyond argument: the twelve plain modules assemble to
//! `TEMPST.LDA` **exactly**, 24,576 of 24,576 bytes over `3000-DFFF`, while the
//! same image against `ALEXEC.LDA` differs in 1,052. Every section also lands
//! precisely on `ALEXEC.MAP`'s table, which is layout evidence the checksum
//! bytes cannot give.
//!
//! The documents concur:
//! `002X1.DAT` names `TEMPEST.LDA` against twelve 2516 parts and `ALEXEC.COM`
//! splits the plain-module link into 2K images at exactly those addresses,
//! while `002X2.DAT` names `ALEXEC.LDA` against six 4K 2532 parts —
//! `TEMPST.DOC`'s version 2A of 12-17-81. Version 1 is dated 8/26/81 and
//! `ALEXEC.MAP` 27-AUG-81, the next day.
//!
//! Note what does *not* discriminate, since it looks as though it should: the
//! ROM self-test at `ALTEST.MAC:364` passes on both images, all twelve blocks.
//! `NROMS=12` lives in the shared `ALCOMN.MAC`, and six 4K devices are still
//! twelve 2K blocks to the software.
//!
//! **The plain set is targeted first**, because `ALEXEC.MAP` states its
//! placement outright where Space Duel's had to be inferred from byte runs.
//!
//! # The two original link lines
//!
//! `ALEXEC.MAP` and `ALEXEC.COM`, 27-AUG-81 — the build targeted here:
//!
//! ```text
//! BIN:ALEXEC,ALEXEC.XX=OBJ:ALWELG,ALSCOR,ALDISP,ALEXEC,ALSOUN,ALVROM/C
//! ALCOIN,ALLANG,ALHARD,ALTEST,ALEARO,ALVGUT
//! ```
//!
//! `TEMPST.DOC`'s version 2A, 12-17-81 — the other one, for the record:
//!
//! ```text
//! ALEXEC/L,ALEXEC/A=ALWELG,ALSCO2,ALDIS2,ALEXEC,ALSOUN,ALVROM,ALCOIN,
//! ALLANG,ALHAR2,ALTES2,ALEARO,ALVGUT
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use chill65_asm::assemble::{Assembler, SourceProvider};

/// Where Tempest's relocatable region begins, per `ALEXEC.MAP`'s Section
/// Summary: `. ABS. 0000 A8B0 ABS,OVR`.
const TEMPEST_ORIGIN: u16 = 0xA8B0;

/// `ALEXEC.MAP`'s Section Summary, verbatim: (module, base, size).
///
/// Eleven rows for twelve modules. `ALWELG` has no `REL,CON` row at all — it
/// contributes to the absolute section only, which is why the relocatable
/// region opens with `ALSCOR`.
const MAP_SECTIONS: [(&str, u16, u16); 11] = [
    ("ALSCOR", 0xA8B0, 0x0906),
    ("ALDISP", 0xB1B6, 0x15EA),
    ("ALEXEC", 0xC7A0, 0x0361),
    ("ALSOUN", 0xCB01, 0x02DD),
    ("ALVROM", 0xCDDE, 0x0146),
    ("ALCOIN", 0xCF24, 0x010D),
    ("ALLANG", 0xD031, 0x06D2),
    ("ALHARD", 0xD703, 0x00DE),
    ("ALTEST", 0xD7E1, 0x05FC),
    ("ALEARO", 0xDDDD, 0x012C),
    ("ALVGUT", 0xDF09, 0x00D3),
];

/// The twelve link units, in `ALEXEC.COM`'s order.
///
/// **The order is load-bearing** — it decides where each section contribution
/// lands, so a permutation links to a different image.
const ROOTS: [&str; 12] = [
    "ALWELG.MAC",
    "ALSCOR.MAC",
    "ALDISP.MAC",
    "ALEXEC.MAC",
    "ALSOUN.MAC",
    "ALVROM.MAC",
    "ALCOIN.MAC",
    "ALLANG.MAC",
    "ALHARD.MAC",
    "ALTEST.MAC",
    "ALEARO.MAC",
    "ALVGUT.MAC",
];

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
/// earlier ones — HLL65's `FND` rewinds the location counter to patch branch
/// operands, so the resolved image is what was burned, not the record stream.
///
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
    /// Every section as placed: (name, base, size), in layout order.
    sections: Vec<(String, u16, u16)>,
}

/// Assemble, and hand back the image **whether or not the assembly was clean**.
///
/// The twelve-module link now reports no errors at all, but a panic on the
/// error path would say only that a fault exists. What the image contains at
/// that point is evidence — which modules landed, where, and against what — so
/// `Assembler` keeps `image` populated when it returns `Err` and the
/// comparison still runs and still reports.
fn build(corpus: &str, roots: &[&str], dirs: &[&str]) -> Built {
    let root = PathBuf::from(corpus);
    let p = Search {
        dirs: dirs.iter().map(|d| root.join(d)).collect(),
    };
    let mut a = Assembler::new(&p);
    // Tempest's relocatable region does not start where Space Duel's does, and
    // this is the one place that difference has to be stated.
    a.section_origin = TEMPEST_ORIGIN;
    let (image, errors) = match a.assemble_units(roots) {
        Ok(img) => (img, Vec::new()),
        Err(errs) => (std::mem::take(&mut a.image), errs),
    };
    let sections: Vec<(String, u16, u16)> = a
        .sec_order
        .iter()
        .map(|n| {
            (
                n.clone(),
                a.sec_base.get(n).copied().unwrap_or(0),
                a.sec_size.get(n).copied().unwrap_or(0),
            )
        })
        .collect();
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
        sections,
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
        eprintln!(
            "  first difference at {addr:04X}: ours {:02X}, oracle {:02X}",
            ours[first], theirs[first]
        );
        eprintln!("  nearest symbol : {}", nearest_symbol(&built.symbols, addr));
        eprintln!(
            "  ours   {:04X}: {}",
            lo as usize + from,
            ours[from..to]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        );
        eprintln!(
            "  oracle {:04X}: {}",
            lo as usize + from,
            theirs[from..to]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        );

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
            eprintln!(
                "    {:04X}-{:04X} ({} bytes)  {}",
                lo as usize + a,
                lo as usize + b,
                b - a + 1,
                nearest_symbol(&built.symbols, lo + *a as u16)
            );
        }
    }
    diffs.len()
}

fn corpus() -> Option<String> {
    std::env::var("CHILL65_CORPUS").ok()
}

/// Which module a section name belongs to, for matching against the map.
///
/// Sections are keyed per unit — the blank one as `~blank@ALSCOR.MAC`, since
/// `blank_section` in `assemble.rs` gives each unit its own — so the map's
/// module name is recovered by taking whatever sits after the `@` and dropping
/// the extension. A named `.CSECT` would arrive without an `@` and is reported
/// as itself, which is the surprise worth seeing if it happens.
fn module_of(section: &str) -> String {
    let tail = section.rsplit('@').next().unwrap_or(section);
    tail.trim_end_matches(".MAC").to_string()
}

/// The twelve modules' sections must land exactly where `ALEXEC.MAP` says.
///
/// This is the half of the oracle question that bytes alone cannot answer. The
/// map records the placement of the plain-module link; if our measured sizes
/// reproduce it, the map describes that build as we assemble it, and the
/// checksum-byte evidence in this file's header gains an independent second
/// witness.
///
/// **This is committed asserting the target, not today's measurement.** Like
/// `gate_sd.rs`'s program test it may fail on arrival; weakening it would leave
/// nothing able to say when the target is met. It is `#[ignore]`d, so a default
/// `cargo test` is unaffected.
#[test]
#[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
fn sections_land_where_the_map_says() {
    let Some(c) = corpus() else {
        return;
    };
    let built = build(&c, &ROOTS, &["."]);
    report_errors(&built.errors);

    eprintln!("\n  measured sections, in layout order:");
    eprintln!("    {:28} {:>6} {:>6}   {:>6} {:>6}", "section", "base", "size", "map", "size");
    for (name, base, size) in &built.sections {
        let m = module_of(name);
        match MAP_SECTIONS.iter().find(|(n, _, _)| *n == m) {
            Some((_, mb, ms)) => eprintln!(
                "    {name:28} {base:>6X} {size:>6X}   {mb:>6X} {ms:>6X}{}",
                if base == mb && size == ms { "" } else { "   <-- differs" }
            ),
            None => eprintln!("    {name:28} {base:>6X} {size:>6X}   {:>6} {:>6}", "-", "-"),
        }
    }
    eprintln!();

    let measured: Vec<(String, u16, u16)> = built
        .sections
        .iter()
        .map(|(n, b, s)| (module_of(n), *b, *s))
        .collect();

    for (name, base, size) in MAP_SECTIONS {
        let got = measured.iter().find(|(n, _, _)| n == name);
        let Some((_, gb, gs)) = got else {
            panic!("{name} contributes no section; the map places it at {base:04X}/{size:04X}");
        };
        assert_eq!(
            (*gb, *gs),
            (base, size),
            "{name}: measured {gb:04X}/{gs:04X}, map says {base:04X}/{size:04X}"
        );
    }

    // ALWELG has no REL,CON row in the map: it is absolute-only, and a
    // relocatable contribution from it would push every other section along.
    assert!(
        !measured.iter().any(|(n, _, _)| n == "ALWELG"),
        "ALWELG contributes a relocatable section; the map gives it none"
    );
}

/// The twelve-module program, `3000-DFFF`, against `TEMPST.LDA`.
///
/// Two windows, because they fail for different reasons and each wants its own
/// readout: `3000-A8AF` is the absolute region — the VG ROM at `3000` and the
/// program from `9000` — and `A8B0-DFFF` is the relocatable one this file's
/// `MAP_SECTIONS` describes.
///
/// **This is committed failing on purpose**, as `gate_sd.rs`'s program test was
/// before it. What it does today is measure: how much of the image is right,
/// where the first difference is, what symbol it sits in, and how the
/// differences are shaped. The assertion is the target and stays as written.
#[test]
#[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
fn program_image_is_byte_identical() {
    let Some(c) = corpus() else {
        return;
    };
    let built = build(&c, &ROOTS, &["."]);
    report_errors(&built.errors);

    // Both oracles, once, so the pairing argued in this file's header is
    // checked against bytes rather than taken on trust. Only the plain-module
    // one is asserted; the other is printed for the contrast it gives.
    let oracle = load_lda(&PathBuf::from(&c).join("TEMPST.LDA"));
    let other = load_lda(&PathBuf::from(&c).join("ALEXEC.LDA"));
    eprintln!("\n-- against ALEXEC.LDA, the starred 2A build (not our target) --");
    compare("OTHER", &built, &other, 0x3000, 0xDFFF);
    eprintln!("\n-- against TEMPST.LDA, the plain-module build --");
    let abs = compare("ABSOLUTE 3000-A8AF", &built, &oracle, 0x3000, 0xA8AF);
    let rel = compare("RELOCATABLE A8B0-DFFF", &built, &oracle, 0xA8B0, 0xDFFF);
    assert_eq!(abs + rel, 0, "program image must match the oracle exactly");
}
