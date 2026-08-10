//! Command-line entry point.
//!
//! ```text
//! chill65-asm <root.MAC> --include-dir DIR [--include-dir DIR2 ...]
//!             [-o out.bin] [--symbols out.sym] [--ir out.ir]
//!             [--base ADDR] [--end ADDR]
//! ```
//!
//! Include directories are searched in order, which is what lets the program be
//! built from `version-3/` while the castle data comes from the root tree — see
//! `integrity.md` §6.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chill65_asm::assemble::{Assembler, SourceProvider};

/// Searches several directories in order, trying each name as written and then
/// with `.MAC` appended.
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

fn usage() -> ! {
    eprintln!(
        "usage: chill65-asm <root.MAC> --include-dir DIR [--include-dir DIR]...\n\
         \x20            [-o OUT.bin] [--symbols OUT.sym] [--base ADDR] [--end ADDR]\n\
         \n\
         Addresses are hexadecimal. --base/--end bound the emitted binary;\n\
         by default the whole written range is emitted."
    );
    std::process::exit(2)
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        usage();
    }

    let mut roots: Vec<String> = Vec::new();
    let mut dirs = Vec::new();
    let mut out = None;
    let mut symfile = None;
    let mut irfile: Option<PathBuf> = None;
    let mut base: Option<u16> = None;
    let mut end: Option<u16> = None;

    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        let next = |i: &mut usize| -> String {
            *i += 1;
            argv.get(*i).cloned().unwrap_or_else(|| usage())
        };
        match a.as_str() {
            "--include-dir" | "-I" => dirs.push(PathBuf::from(next(&mut i))),
            "-o" | "--output" => out = Some(PathBuf::from(next(&mut i))),
            "--symbols" => symfile = Some(PathBuf::from(next(&mut i))),
            "--ir" => irfile = Some(PathBuf::from(next(&mut i))),
            "--base" => base = u16::from_str_radix(&next(&mut i), 16).ok(),
            "--end" => end = u16::from_str_radix(&next(&mut i), 16).ok(),
            "-h" | "--help" => usage(),
            _ if a.starts_with('-') => usage(),
            _ => roots.push(a.clone()),
        }
        i += 1;
    }

    if roots.is_empty() {
        usage()
    }
    let root = roots.join(" + ");
    if dirs.is_empty() {
        dirs.push(PathBuf::from("."));
    }

    let provider = Search { dirs };
    let mut asm = Assembler::new(&provider);
    let refs: Vec<&str> = roots.iter().map(|s| s.as_str()).collect();
    let result = asm.assemble_units(&refs);

    match result {
        Ok(image) => {
            for w in &asm.warnings {
                eprintln!("warning: {w}");
            }
            report(&root, &asm, &image);
            if std::env::var("CHILL65_ANALYSE").is_ok() {
                analyse(&asm);
            }
            if let Some(path) = out {
                write_image(&image, base, end, &path);
            }
            if let Some(path) = symfile {
                write_symbols(&asm, &path);
            }
            if let Some(path) = &irfile {
                // Game-derived when run against the corpus: write under
                // target/, never commit (plan section 9).
                if let Err(e) = std::fs::write(path, asm.ir.dump()) {
                    eprintln!("cannot write {}: {e}", path.display());
                    std::process::exit(1);
                }
                eprintln!(
                    "wrote {} IR events to {}",
                    asm.ir.events.len(),
                    path.display()
                );
            }
        }
        Err(errors) => {
            eprintln!("{root}: {} error(s)", errors.len());
            // Distinct messages first, so a repeated fault does not bury the rest.
            let mut seen = std::collections::BTreeMap::new();
            for e in &errors {
                *seen.entry(e.clone()).or_insert(0usize) += 1;
            }
            // Category histogram first: 200 distinct messages is unreadable,
            // but the shape of them is not.
            let mut cats: BTreeMap<String, usize> = BTreeMap::new();
            for e in &errors {
                let stripped = e
                    .split_once(": ")
                    .map(|(_, r)| r.to_string())
                    .unwrap_or_else(|| e.clone());
                let key = stripped
                    .split_whitespace()
                    .take(4)
                    .collect::<Vec<_>>()
                    .join(" ");
                *cats.entry(key).or_insert(0) += 1;
            }
            let mut cv: Vec<_> = cats.into_iter().collect();
            cv.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            eprintln!("  --- by category ---");
            for (k, n) in cv.iter().take(15) {
                eprintln!("  {n:5}  {k}");
            }
            eprintln!("  --- distinct ---");
            for (msg, n) in seen.iter().take(500) {
                if *n > 1 {
                    eprintln!("  {msg}   [x{n}]");
                } else {
                    eprintln!("  {msg}");
                }
            }
            if seen.len() > 40 {
                eprintln!("  ... and {} more distinct", seen.len() - 40);
            }
            std::process::exit(1);
        }
    }
}

/// Post-expansion analysis: the self-modifying-code re-check the Phase 0 gate
/// promised, and the open-question O3 measurement.
fn analyse(asm: &Assembler) {
    // Every store's resolved target, checked against the ROM window. The Phase 0
    // scan could only read source text; this sees the fully expanded stream.
    let into_rom: Vec<_> = asm
        .store_targets
        .iter()
        .filter(|(_, addr, _)| *addr >= 0xA000)
        .collect();
    eprintln!(
        "\nSMC re-check (post-expansion): {} stores with a resolved target, {} into ROM (A000-FFFF)",
        asm.store_targets.len(),
        into_rom.len()
    );
    for (op, addr, chain) in into_rom.iter().take(20) {
        eprintln!("    {op} -> {addr:04X}   via {chain}");
    }

    // O3: structured control flow versus bare branches.
    const HLL: &[&str] = &[
        "IFEQ", "IFNE", "IFCC", "IFCS", "IFMI", "IFPL", "IFVC", "IFVS", "IF", "ELSE", "ENDIF",
        "THEN", "ENDC", "BEGIN", "CCEND", "CSEND", "EQEND", "NEEND", "MIEND", "PLEND", "VCEND",
        "VSEND", "CCCONT", "CSCONT", "EQCONT", "NECONT", "MICONT", "PLCONT", "VCCONT", "VSCONT",
        "CONTINUE", "END",
    ];
    let mut structured = 0usize;
    let mut per: Vec<(String, usize)> = Vec::new();
    for (name, n) in &asm.macro_uses {
        if HLL.contains(&name.as_str()) {
            structured += n;
            per.push((name.clone(), *n));
        }
    }
    per.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let total = asm.branches_structured + asm.branches_direct + asm.branches_other_macro;
    eprintln!("\nO3: {structured} structured-control-flow constructs invoked");
    eprintln!("    branches by origin, of {total}:");
    eprintln!(
        "      HLL65F structured : {:5}  ({:.1}%)",
        asm.branches_structured,
        100.0 * asm.branches_structured as f64 / total as f64
    );
    eprintln!(
        "      other macros      : {:5}  ({:.1}%)",
        asm.branches_other_macro,
        100.0 * asm.branches_other_macro as f64 / total as f64
    );
    eprintln!(
        "      hand-written      : {:5}  ({:.1}%)",
        asm.branches_direct,
        100.0 * asm.branches_direct as f64 / total as f64
    );
    eprintln!("    most used:");
    for (n, c) in per.iter().take(10) {
        eprintln!("      {n:<8} {c}");
    }
}

fn report(root: &str, asm: &Assembler, image: &BTreeMap<u16, u8>) {
    let (lo, hi) = match (image.keys().next(), image.keys().next_back()) {
        (Some(a), Some(b)) => (*a, *b),
        _ => (0, 0),
    };
    eprintln!(
        "{root}: {} symbols, {} macros, {} bytes emitted, {:04X}-{:04X}, {} unhandled",
        asm.locals.len() + asm.globals.len(),
        asm.macros.len(),
        image.len(),
        lo,
        hi,
        asm.unhandled
    );
    let span = (hi as usize).saturating_sub(lo as usize) + 1;
    if image.len() < span {
        eprintln!("  ({} bytes in range never written)", span - image.len());
    }
}

fn write_image(image: &BTreeMap<u16, u8>, base: Option<u16>, end: Option<u16>, path: &PathBuf) {
    let lo = base.or_else(|| image.keys().next().copied()).unwrap_or(0);
    let hi = end.or_else(|| image.keys().next_back().copied()).unwrap_or(0);
    let bytes: Vec<u8> = (lo..=hi).map(|a| *image.get(&a).unwrap_or(&0)).collect();
    if let Err(e) = std::fs::write(path, &bytes) {
        eprintln!("cannot write {}: {e}", path.display());
        std::process::exit(1);
    }
    eprintln!("wrote {} bytes {:04X}-{:04X} to {}", bytes.len(), lo, hi, path.display());
}

fn write_symbols(asm: &Assembler, path: &PathBuf) {
    let mut lines: Vec<String> = asm
        .globals
        .iter()
        .map(|(k, v)| format!("{v:04X} {k} [global]"))
        .collect();
    for (unit, table) in &asm.unit_locals {
        for (k, v) in table {
            lines.push(format!("{v:04X} {k} [{unit}]"));
        }
    }
    lines.sort();
    if let Err(e) = std::fs::write(path, lines.join("\n") + "\n") {
        eprintln!("cannot write {}: {e}", path.display());
        std::process::exit(1);
    }
    eprintln!("wrote {} symbols to {}", lines.len(), path.display());
}
