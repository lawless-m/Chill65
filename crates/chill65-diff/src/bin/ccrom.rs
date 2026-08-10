//! `ccrom` — rebuild MAME's `ccastles3` ROM set from the game source.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles cargo run -p chill65-diff --bin ccrom
//! ```
//!
//! Prints a line per device and exits nonzero if any CRC-32 disagrees with the
//! one MAME expects. The archive lands under `target/`, which is gitignored:
//! these are game-derived bytes and must never be committed (plan §9).

use std::path::PathBuf;
use std::process::ExitCode;

use chill65_diff::images::corpus;
use chill65_diff::romset::build_and_write;

const USAGE: &str = "\
usage: ccrom [options]

Rebuilds the eleven devices of MAME's ccastles3 set from the game source and
checks each against the CRC-32 MAME expects.

options:
  --corpus DIR     game source root (default: $CHILL65_CORPUS)
  -h, --help       print this message
";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("ccrom: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool, String> {
    let mut args = std::env::args().skip(1);
    let mut root: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(true);
            }
            "--corpus" => {
                root = Some(args.next().ok_or("--corpus needs a path")?.into());
            }
            other => return Err(format!("unknown option {other}")),
        }
    }

    let root = root
        .or_else(corpus)
        .ok_or("no corpus: pass --corpus or set CHILL65_CORPUS")?;

    let (set, path) = build_and_write(&root)?;
    for d in &set.devices {
        println!(
            "{} {:22} {:5} bytes  {:08X}{}",
            if d.ok() { "PASS" } else { "FAIL" },
            d.name,
            d.len,
            d.got,
            if d.ok() {
                String::new()
            } else {
                format!("  (expected {:08X})", d.want)
            }
        );
    }

    if set.all_ok() {
        println!(
            "\nall {} devices verified -> {}",
            set.devices.len(),
            path.display()
        );
    } else {
        let bad = set.devices.iter().filter(|d| !d.ok()).count();
        eprintln!("\n{bad} of {} devices do not match", set.devices.len());
    }
    Ok(set.all_ok())
}
