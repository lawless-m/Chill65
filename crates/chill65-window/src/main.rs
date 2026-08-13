//! `chill65-window` — Space Duel on a modelled vector tube.

use std::path::PathBuf;
use std::process::ExitCode;

use chill65_window::{capture, window};

const USAGE: &str = "\
usage: chill65-window [options]

  (no arguments)     open the window and play

options:
  --self-check       report the GPU adapter and the renderer's shaders
  --capture FILE     play a scripted game and write a .btr0 beam trace
  --seconds N        how long to capture (default 10)
  --help

The game's source directory must be given in CHILL65_CORPUS. No ROMs are
distributed with this project; see the README.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("--capture") => run_capture(&args[1..]),
        Some("--self-check") => window::self_check(),
        None => corpus().and_then(window::run),
        Some(other) => Err(format!("unknown argument {other}\n\n{USAGE}")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("chill65-window: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The corpus directory, or a message explaining what is missing.
fn corpus() -> Result<PathBuf, String> {
    std::env::var("CHILL65_CORPUS")
        .map(PathBuf::from)
        .map_err(|_| {
            "CHILL65_CORPUS is not set — point it at the Space Duel source \
             directory. No ROMs are distributed with this project."
                .to_owned()
        })
}

fn run_capture(rest: &[String]) -> Result<(), String> {
    let mut out: Option<PathBuf> = None;
    let mut seconds = 10.0f64;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => {
                let v = args.next().ok_or("--seconds needs a number")?;
                seconds = v
                    .parse()
                    .map_err(|_| format!("--seconds wants a number, not {v}"))?;
            }
            other if out.is_none() && !other.starts_with("--") => {
                out = Some(PathBuf::from(other));
            }
            other => return Err(format!("unknown argument {other}\n\n{USAGE}")),
        }
    }
    let out = out.ok_or_else(|| format!("--capture needs a file\n\n{USAGE}"))?;
    if seconds <= 0.0 {
        return Err(format!("--seconds must be positive, not {seconds}"));
    }
    capture::run(&corpus()?, &out, seconds)
}
