//! `chill65-window` — Space Duel or Tempest on a modelled vector tube.

use std::path::PathBuf;
use std::process::ExitCode;

use chill65_window::{capture, window};
use tube_renderer::View;

const USAGE: &str = "\
usage: chill65-window [options]

  (no arguments)     open the window and play

options:
  --self-check       report the GPU adapter and the renderer's shaders
  --capture FILE     play a scripted game and write a .btr0 beam trace
  --seconds N        how long to capture (default 10)
  --view NAME        which readout to open on: beauty (default), fast, slow,
                     deposit, energy, samples. Tab cycles them while running.
                     `energy` is the legible one until Trexy's beauty tonemap
                     can hold a picture with a bright explosion and a faint
                     web in the same frame.
  --help

The game's source directory must be given in CHILL65_CORPUS, and which game it
is comes from what is in it — Space Duel and Tempest are both understood. No
ROMs are distributed with this project; see the README.
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
        None => corpus().and_then(|c| window::run(c, View::default())),
        Some("--view") => view(&args[1..]).and_then(|v| Ok((corpus()?, v)))
            .and_then(|(c, v)| window::run(c, v)),
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

/// The readout named after `--view`, or the names it could have been.
fn view(rest: &[String]) -> Result<View, String> {
    let name = rest.first().ok_or("--view needs a name")?;
    View::from_name(name).ok_or_else(|| {
        let names: Vec<&str> = View::ALL.iter().map(|v| v.name()).collect();
        format!("no readout called {name} — try one of {}", names.join(", "))
    })
}

/// The corpus directory, or a message explaining what is missing.
fn corpus() -> Result<PathBuf, String> {
    std::env::var("CHILL65_CORPUS")
        .map(PathBuf::from)
        .map_err(|_| {
            "CHILL65_CORPUS is not set — point it at the Space Duel or Tempest \
             source directory. No ROMs are distributed with this project."
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
