//! `ccdiff` — the differential harness driver.
//!
//! ```text
//! ccdiff run --trace T [--frames N] --out DIR [--pixels] [--prog P --data D]
//! ccdiff compare STREAM_A STREAM_B
//! ccdiff localise --trace T --sym S --prog-a P --data-a D --prog-b P --data-b D
//! ```
//!
//! `run` plays a trace on our runtime and writes the per-frame hash stream;
//! `compare` says where two such streams first disagree; `localise` runs two
//! image pairs over one trace and names the routines behind any divergence.
//!
//! No third-party argument parser, matching the rest of the workspace.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chill65_diff::diverge::{compare, read_stream, write_stream};
use chill65_diff::images::{build_images, corpus};
use chill65_diff::localise::localise;
use chill65_diff::symbols::Symbols;
use chill65_diff::trace::Trace;
use chill65_diff::{OurRuntime, Reference};

const USAGE: &str = "\
usage: ccdiff <command> [options]

commands:
  run       play a trace on our runtime and write its per-frame hash stream
  compare   report where two hash streams first disagree
  localise  run two image pairs over one trace and name the routines behind
            any divergence

run options:
  --trace FILE     trace to play (required)
  --frames N       frames to run (default: the trace's length)
  --out DIR        directory for hashes.txt, and frames if --pixels (required)
  --pixels         also dump each frame as raw 256x232 RGB24
  --prog FILE      program image; default: assembled from CHILL65_CORPUS
  --data FILE      castle data image; likewise

compare:
  ccdiff compare A B   exits 0 whether or not they diverge; nonzero only on
                       an I/O or format error

localise options:
  --trace FILE     trace both sides play (required)
  --frames N       frames to run (default: the trace's length)
  --sym FILE       symbol table from chill65-asm --symbols (required)
  --prog-a, --data-a, --prog-b, --data-b   the two image pairs (required)

options:
  -h, --help       print this message

Everything written goes under the directory you name. Frame dumps carry
game-derived bytes and must never be committed (plan section 9).
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(());
    }
    let Some(command) = args.first() else {
        print!("{USAGE}");
        return Ok(());
    };

    match command.as_str() {
        "run" => cmd_run(&args[1..]),
        "compare" => cmd_compare(&args[1..]),
        "localise" => cmd_localise(&args[1..]),
        other => Err(format!("unknown command {other}")),
    }
}

/// Fetch the value following `flag`.
fn value(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn cmd_run(args: &[String]) -> Result<(), String> {
    let mut trace_path: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut frames: Option<u32> = None;
    let mut prog_path: Option<PathBuf> = None;
    let mut data_path: Option<PathBuf> = None;
    let mut pixels = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--trace" => trace_path = Some(value(args, &mut i, "--trace")?.into()),
            "--out" => out = Some(value(args, &mut i, "--out")?.into()),
            "--prog" => prog_path = Some(value(args, &mut i, "--prog")?.into()),
            "--data" => data_path = Some(value(args, &mut i, "--data")?.into()),
            "--pixels" => pixels = true,
            "--frames" => {
                let v = value(args, &mut i, "--frames")?;
                frames = Some(v.parse().map_err(|_| format!("bad frame count {v:?}"))?);
            }
            other => return Err(format!("unknown option {other}")),
        }
        i += 1;
    }

    let trace_path = trace_path.ok_or("run needs --trace")?;
    let out = out.ok_or("run needs --out")?;

    let text = std::fs::read_to_string(&trace_path)
        .map_err(|e| format!("{}: {e}", trace_path.display()))?;
    let trace = Trace::parse(&text).map_err(|e| format!("{}: {e}", trace_path.display()))?;
    let frames = frames.unwrap_or(trace.len() as u32);

    let (prog, data) = match (prog_path, data_path) {
        (Some(p), Some(d)) => (
            std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?,
            std::fs::read(&d).map_err(|e| format!("{}: {e}", d.display()))?,
        ),
        (None, None) => {
            let corpus = corpus().ok_or(
                "no images: pass --prog and --data, or set CHILL65_CORPUS to assemble them",
            )?;
            let images = build_images(&corpus)?;
            (images.prog, images.data)
        }
        _ => return Err("--prog and --data must be given together".into()),
    };

    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;

    // No motion ROMs: `run` takes whatever images it is given and has
    // no third one to take. Sprites are absent from what it dumps.
    let mut player = OurRuntime::new(prog, data, None);
    let run = player.run(&trace, frames, pixels)?;

    let hashes: Vec<u64> = run.iter().map(|f| f.hash).collect();
    let stream_path = out.join("hashes.txt");
    write_stream(&stream_path, &hashes)?;
    println!("wrote {} frames to {}", hashes.len(), stream_path.display());

    if pixels {
        for (k, frame) in run.iter().enumerate() {
            let rgb = frame.rgb.as_ref().expect("--pixels asked for them");
            let path = out.join(format!("frame-{k:05}.rgb"));
            std::fs::write(&path, rgb).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        println!("wrote {} raw RGB frames to {}", run.len(), out.display());
    }
    Ok(())
}

fn cmd_localise(args: &[String]) -> Result<(), String> {
    let mut trace_path: Option<PathBuf> = None;
    let mut sym_path: Option<PathBuf> = None;
    let mut frames: Option<u32> = None;
    let mut images: [Option<PathBuf>; 4] = [None, None, None, None];

    let mut i = 0;
    while i < args.len() {
        let flag = args[i].clone();
        match flag.as_str() {
            "--trace" => trace_path = Some(value(args, &mut i, "--trace")?.into()),
            "--sym" => sym_path = Some(value(args, &mut i, "--sym")?.into()),
            "--prog-a" => images[0] = Some(value(args, &mut i, &flag)?.into()),
            "--data-a" => images[1] = Some(value(args, &mut i, &flag)?.into()),
            "--prog-b" => images[2] = Some(value(args, &mut i, &flag)?.into()),
            "--data-b" => images[3] = Some(value(args, &mut i, &flag)?.into()),
            "--frames" => {
                let v = value(args, &mut i, "--frames")?;
                frames = Some(v.parse().map_err(|_| format!("bad frame count {v:?}"))?);
            }
            other => return Err(format!("unknown option {other}")),
        }
        i += 1;
    }

    let trace_path = trace_path.ok_or("localise needs --trace")?;
    let sym_path = sym_path.ok_or("localise needs --sym")?;
    let mut loaded = Vec::new();
    for (path, flag) in images
        .iter()
        .zip(["--prog-a", "--data-a", "--prog-b", "--data-b"])
    {
        let path = path.as_ref().ok_or(format!("localise needs {flag}"))?;
        loaded.push(std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?);
    }

    let text = std::fs::read_to_string(&trace_path)
        .map_err(|e| format!("{}: {e}", trace_path.display()))?;
    let trace = Trace::parse(&text).map_err(|e| format!("{}: {e}", trace_path.display()))?;
    let frames = frames.unwrap_or(trace.len() as u32);
    let symbols = Symbols::load(&sym_path)?;

    let mut b_images = loaded.split_off(2);
    let mut a = OurRuntime::new(loaded.remove(0), loaded.remove(0), None);
    let mut b = OurRuntime::new(b_images.remove(0), b_images.remove(0), None);

    println!("{}", localise(&mut a, &mut b, &trace, frames, &symbols)?);
    Ok(())
}

fn cmd_compare(args: &[String]) -> Result<(), String> {
    if args.len() != 2 {
        return Err(format!("compare needs two stream files, got {}", args.len()));
    }
    let a = read_stream(Path::new(&args[0]))?;
    let b = read_stream(Path::new(&args[1]))?;
    println!("{}", compare(&a, &b));
    Ok(())
}
