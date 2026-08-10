//! `ccdiff` — the differential harness driver.
//!
//! A skeleton at this stage: it parses arguments and prints usage. The
//! subcommands that do the work arrive in later slices.
//!
//! No third-party argument parser, matching the rest of the workspace.

use std::process::ExitCode;

const USAGE: &str = "\
usage: ccdiff [options]

options:
  -h, --help       print this message
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ccdiff: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = std::env::args().skip(1);

    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }

    print!("{USAGE}");
    Ok(())
}
