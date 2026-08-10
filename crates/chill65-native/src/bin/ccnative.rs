//! `ccnative` — measure and check the migration.
//!
//! ```text
//! ccnative coverage --trace FILE [--frames N]
//! ccnative verify   --trace FILE [--frames N]
//! ```
//!
//! `coverage` answers "how much of the running code is compiled", and `verify`
//! answers "is it still right". Both need `CHILL65_CORPUS`.
//!
//! # The metric
//!
//! **Compiled instructions divided by total executed instructions**, over a
//! trace played end to end. Executed instructions, not routines and not bytes:
//! they weight a routine by how much it actually runs. Counting routines would
//! let a hundred cold ones be migrated and majority declared; counting bytes
//! would do the same for a large table-driven routine that never executes.
//!
//! No third-party argument parser, matching the rest of the workspace.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use chill65_diff::images::{build_images, corpus};
use chill65_diff::localise::attribute;
use chill65_diff::symbols::Symbols;
use chill65_diff::trace::Trace;
use chill65_diff::OurRuntime;
use chill65_native::Registry;
use chill65_runtime::frame::{Compiled, NoCompiled};
use chill65_runtime::{Cpu, Machine};

const USAGE: &str = "\
usage: ccnative <command> --trace FILE [--frames N]

commands:
  coverage   report how many executed instructions are compiled
  verify     run the trace with dispatch off and on and require identical
             pictures; on divergence, name the frame and the routines

options:
  --trace FILE     trace to play (required)
  --frames N       frames to run (default: the trace's length)
  -h, --help       print this message

CHILL65_CORPUS must point at the game source.
";

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("ccnative: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Counts what the interpreter executes, by program counter.
///
/// `Compiled::enter` is offered at **every** instruction boundary, so anything
/// it declines is about to be interpreted. That makes a wrapper the natural
/// place to profile, with no further instrumentation in the runtime.
struct Profiler {
    inner: Registry,
    interpreted: HashMap<u16, u64>,
}

impl Profiler {
    fn new(inner: Registry) -> Profiler {
        Profiler {
            inner,
            interpreted: HashMap::new(),
        }
    }
}

impl Compiled for Profiler {
    fn enter(&mut self, cpu: &mut Cpu, machine: &mut Machine, deadline: u64) -> bool {
        let pc = cpu.pc;
        if self.inner.enter(cpu, machine, deadline) {
            return true;
        }
        *self.interpreted.entry(pc).or_default() += 1;
        false
    }

    fn compiled_instructions(&self) -> u64 {
        self.inner.compiled_instructions()
    }
}

struct Args {
    trace: PathBuf,
    frames: Option<u32>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut trace = None;
    let mut frames = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--trace" => {
                i += 1;
                trace = Some(PathBuf::from(
                    args.get(i).ok_or("--trace needs a path")?.clone(),
                ));
            }
            "--frames" => {
                i += 1;
                let v = args.get(i).ok_or("--frames needs a value")?;
                frames = Some(v.parse().map_err(|_| format!("bad frame count {v:?}"))?);
            }
            other => return Err(format!("unknown option {other}")),
        }
        i += 1;
    }
    Ok(Args {
        trace: trace.ok_or("--trace is required")?,
        frames,
    })
}

fn load_trace(path: &PathBuf) -> Result<Trace, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Trace::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn setup(args: &Args) -> Result<(OurRuntime, Symbols, Trace, u32), String> {
    let game = corpus().ok_or(
        "CHILL65_CORPUS is not set; it must point at the game source for either command",
    )?;
    let images = build_images(&game)?;
    let symbols = Symbols::load(&images.sym)?;
    let trace = load_trace(&args.trace)?;
    let frames = args.frames.unwrap_or(trace.len() as u32);
    Ok((
        OurRuntime::new(images.prog, images.data),
        symbols,
        trace,
        frames,
    ))
}

fn run() -> Result<bool, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() || argv.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return Ok(true);
    }
    match argv[0].as_str() {
        "coverage" => coverage(&parse(&argv[1..])?),
        "verify" => verify(&parse(&argv[1..])?),
        other => Err(format!("unknown command {other}")),
    }
}

fn coverage(args: &Args) -> Result<bool, String> {
    let (mut ours, symbols, trace, frames) = setup(args)?;
    let mut profiler = Profiler::new(Registry::game());
    let play = ours.play_with(&trace, frames, &mut profiler)?;

    let total = play.compiled + play.interpreted;
    let share = if total == 0 {
        0.0
    } else {
        100.0 * play.compiled as f64 / total as f64
    };
    println!(
        "{}: {frames} frames\n  {total} instructions executed\n  {} compiled ({share:.1}%)\n  \
         {} interpreted",
        args.trace.display(),
        play.compiled,
        play.interpreted
    );

    // What to migrate next: the routines the interpreter spends most of its
    // time in. Attribution is by the routine whose span *contains* the program
    // counter, not by the nearest preceding symbol — that would name local
    // labels like `~L326$10`, which are inside routines rather than being any,
    // and so cannot be migrated.
    let containing = |pc: u16| -> String {
        chill65_native::game::ALL_ROUTINES
            .iter()
            .find(|(_, lo, hi)| pc >= *lo && pc < *hi)
            .map(|(n, ..)| (*n).to_string())
            .unwrap_or_else(|| format!("{} (no routine)", symbols.routine_for(pc)))
    };

    // Compiled and interpreted are counted separately per routine, because a
    // routine can be both: dispatch fires only at a routine's *entry*, so one
    // reached by falling through, or by a branch from elsewhere, keeps
    // interpreting however thoroughly it has been lowered. Collapsing the two
    // into one "status" hides exactly the thing worth seeing.
    let mut by_routine: HashMap<String, (u64, u64)> = HashMap::new();
    for (pc, n) in &profiler.interpreted {
        by_routine.entry(containing(*pc)).or_default().1 += n;
    }
    for (entry, n) in profiler.inner.per_routine() {
        by_routine.entry(containing(*entry)).or_default().0 += n;
    }

    let mut ranked: Vec<_> = by_routine.into_iter().collect();
    ranked.sort_by_key(|(name, (c, i))| (std::cmp::Reverse(c + i), name.clone()));
    println!("\n  executed  compiled  interpreted  routine");
    for (name, (c, i)) in ranked.iter().take(15) {
        println!("  {:8}  {c:8}  {i:11}  {name}", c + i);
    }
    Ok(true)
}

fn verify(args: &Args) -> Result<bool, String> {
    let (mut ours, symbols, trace, frames) = setup(args)?;

    let interpreted = ours.play_with(&trace, frames, &mut NoCompiled)?;
    let mut registry = Registry::game();
    let dispatched = ours.play_with(&trace, frames, &mut registry)?;

    let diverged = interpreted
        .hashes
        .iter()
        .zip(&dispatched.hashes)
        .position(|(a, b)| a != b);

    let Some(frame) = diverged else {
        println!(
            "{}: identical over {frames} frames ({} routines compiled, \
             {} of {} instructions)",
            args.trace.display(),
            registry.len(),
            dispatched.compiled,
            dispatched.compiled + dispatched.interpreted
        );
        return Ok(true);
    };

    // A divergence is localised the same way any other is: both sides are ours,
    // both carry write logs, so blame merges from both.
    println!("{}: DIVERGED at frame {frame}", args.trace.display());
    let mine = ours.snapshot(&trace, frame as u32)?;
    let theirs = ours.snapshot_with(&trace, frame as u32, &mut Registry::game())?;
    let (differing, unattributed, blame) = attribute(
        &mine.rgb,
        &theirs.rgb,
        &[&mine.writers, &theirs.writers],
        &symbols,
    );
    println!("  {differing} pixels differ, {unattributed} unattributed");
    for b in blame.iter().take(10) {
        println!("  {:8}  {}", b.pixels, b.symbol);
    }
    println!(
        "\nA compiled routine must be indistinguishable from interpreting it. \
         Fix the emitter, or take the routine out of routines.txt."
    );
    Ok(false)
}
