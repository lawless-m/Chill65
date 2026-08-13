//! Export Space Duel's beam as a `BTR0` trace, for a tube renderer to draw.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/space-duel \
//!   cargo test -p chill65-runtime --test trace_sd -- --ignored --nocapture
//! ```
//!
//! # What this produces, and what it is for
//!
//! `crate::btr0` writes the format; this decides what to put in it. The machine
//! gives us [`BeamMove`]s — where the beam went, lit or blanked — and the trace
//! wants normalised deflection, linear-light drive and time. The conversion is
//! this file. The deflection scale is evidenced (`vg::FULL_SCALE_X`); the beam
//! rate and the drive curve are not, and both are marked where they sit.
//!
//! The output is game-derived, so it is written under `target/` and never
//! committed — plan §9, as for every other artefact the corpus produces.
//!
//! # Why the beam and not the segments
//!
//! A picture is made of lit strokes; a display is driven by a beam that is
//! mostly *not* lit and still takes time to move. The trace format says so
//! outright — blanked travel must appear as zero-drive samples, not be omitted
//! — because that travel is time during which the phosphor decays. So this
//! exports `m.beam`, which `trace_beam` fills, rather than `m.segments`.
//!
//! # Note on the cocktail latch
//!
//! `out_latch` bits 6 and 7 invert X and Y. They are not applied here; that is
//! a separate piece of work on the machine's side, not the exporter's.

mod common;

use chill65_runtime::btr0::{Trace, DEFAULT_EPSILON, DISCONTINUITY};
use chill65_runtime::sd::{run_frame_sd, SdMachine, CPU_HZ, CYCLES_PER_FRAME};
use chill65_runtime::vg::{intensity_drive, BeamMove, FULL_SCALE_X, FULL_SCALE_Y};
use chill65_runtime::Cpu;

use common::{build_sd_program, build_sd_ship, corpus, SD_PROG_LEN};

/// Frames to run before exporting. The self-test finishes around frame 98 and
/// drawing starts at 99 (`boot_sd.rs`), so this is well into attract proper.
const SKIP: u32 = 300;

/// Frames to export.
const TAKE: u32 = 20;

/// Normalisation is **per axis**, from the evidenced full-scale deflection in
/// [`chill65_runtime::vg`] — 256 by 192 generator units, from the XY board's
/// own diagnostic.
///
/// Per axis and not a single factor, because the trace format does not encode
/// aspect ratio: x and y each run to ±1 and the renderer's tube profile
/// supplies the 4:3. Scaling both by the same number would leave the picture
/// short vertically and then let the profile stretch it, which is the same
/// mistake made twice.
fn norm_x(v: i32) -> f32 {
    v as f32 / FULL_SCALE_X as f32
}

fn norm_y(v: i32) -> f32 {
    v as f32 / FULL_SCALE_Y as f32
}

/// Generator ticks per second — the beam's speed.
///
/// **UNVERIFIED, fitted.** `BeamMove::ticks` explains the tick model; this is
/// the rate those ticks run at, and nothing in the game source fixes it. The
/// value is 1.5 MHz because the busiest exported frame must finish inside a
/// frame period, which the test asserts. Note that a *uniform* rate cancels
/// out of a phosphor model's overall brightness — what this shapes is the
/// relative brightness of a long stroke against a short one.
const TICKS_PER_SECOND: f64 = 1_500_000.0;

/// Z-axis rise and fall, seconds.
///
/// **UNVERIFIED.** Blanking is not instant, and the trace interpolates drive
/// linearly between samples, so a stroke needs samples bracketing its edges
/// tightly or the light bleeds into the blanked travel either side.
const Z_EDGE: f32 = 0.5e-6;

const PRODUCER: &str = "chill65/space-duel";

/// Colour index to RGB.
///
/// `AST2RD.MAC:244-251` gives the whole table: `BLACK==0`, `BLUE==1`,
/// `GREEN==2`, `GRBLUE==3`, `RED==4`, `VIOLET==5`, `YELLOW==6`, `WHITE==7`.
/// So bit 0 is blue, bit 1 green, bit 2 red. **UNVERIFIED:** bit 3 of the
/// colour field, which no attract-mode word has ever been seen to set.
fn colour_rgb(color: u8) -> [f32; 3] {
    [
        ((color >> 2) & 1) as f32,
        ((color >> 1) & 1) as f32,
        (color & 1) as f32,
    ]
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// One exported frame's worth of measurements, for the report.
#[derive(Default)]
struct Stats {
    lit: usize,
    blanked: usize,
    jumps: usize,
    ticks: u64,
}

#[test]
#[ignore = "needs CHILL65_CORPUS"]
fn attract_exports_a_beam_trace() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let skip = env_u32("CHILL65_TRACE_SKIP", SKIP);
    let take = env_u32("CHILL65_TRACE_FRAMES", TAKE);
    let out = std::env::var("CHILL65_TRACE_OUT").unwrap_or_else(|_| {
        common::workspace_root()
            .join("target/sd-attract.btr0")
            .display()
            .to_string()
    });

    let ship = build_sd_ship(&c);
    let image = build_sd_program(&c).bytes(0x0000, SD_PROG_LEN);

    let mut m = SdMachine::new();
    m.load_roms(&ship, &image).expect("load");
    m.set_options(0, 0);
    m.self_test = false;
    m.trace_beam = true;

    let mut cpu = Cpu::new();
    cpu.reset(&mut m);
    for _ in 0..skip {
        run_frame_sd(&mut cpu, &mut m).expect("frame");
    }

    let frame_seconds = CYCLES_PER_FRAME as f64 / CPU_HZ as f64;
    let refresh_hz = CPU_HZ as f32 / CYCLES_PER_FRAME as f32;
    let mut trace = Trace::new(PRODUCER, DEFAULT_EPSILON, refresh_hz).expect("producer id");

    let mut per_frame: Vec<Stats> = Vec::with_capacity(take as usize);
    let mut segments: Vec<usize> = Vec::with_capacity(take as usize);
    let (mut lo_x, mut hi_x, mut lo_y, mut hi_y) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    let mut colours = [0usize; 16];
    let mut intensities = [0usize; 8];
    let mut t = 0.0f32;

    for frame in 0..take {
        run_frame_sd(&mut cpu, &mut m).expect("frame");
        segments.push(m.segments.len());
        let mut stats = Stats::default();

        // The GO strobe centres the beam, so each frame opens with a jump to
        // the middle: nothing is deposited across the gap from wherever the
        // last frame left off.
        t = t.max(frame as f32 * frame_seconds as f32) + Z_EDGE;
        push(&mut trace, 0.0, 0.0, [0.0; 3], t, DISCONTINUITY);

        for mv in &m.beam {
            if mv.jump {
                stats.jumps += 1;
                t += Z_EDGE;
                push(&mut trace, norm_x(mv.x1), norm_y(mv.y1), [0.0; 3], t, DISCONTINUITY);
                continue;
            }

            let dt = ((mv.ticks() as f64 / TICKS_PER_SECOND) as f32).max(Z_EDGE);
            stats.ticks += mv.ticks() as u64;

            if mv.intensity == 0 {
                // Blanked travel: real movement, no light.
                stats.blanked += 1;
                t += dt;
                push(&mut trace, norm_x(mv.x1), norm_y(mv.y1), [0.0; 3], t, 0);
                continue;
            }

            stats.lit += 1;
            colours[mv.color as usize] += 1;
            intensities[mv.intensity as usize] += 1;
            for (x, y) in [(mv.x0, mv.y0), (mv.x1, mv.y1)] {
                lo_x = lo_x.min(x);
                hi_x = hi_x.max(x);
                lo_y = lo_y.min(y);
                hi_y = hi_y.max(y);
            }

            let drive = beam_drive(mv);
            // Z on where the stroke starts, sweep, Z off where it ends. The
            // preceding sample is always zero-drive at this same point, so the
            // rise is a fixed-position edge and never a lit smear.
            t += Z_EDGE;
            push(&mut trace, norm_x(mv.x0), norm_y(mv.y0), drive, t, 0);
            t += dt;
            push(&mut trace, norm_x(mv.x1), norm_y(mv.y1), drive, t, 0);
            t += Z_EDGE;
            push(&mut trace, norm_x(mv.x1), norm_y(mv.y1), [0.0; 3], t, 0);
        }

        per_frame.push(stats);
    }

    trace.write(&out).expect("write trace");

    let busiest = per_frame.iter().map(|s| s.ticks).max().unwrap_or(0);
    let busy_seconds = busiest as f64 / TICKS_PER_SECOND;
    let lit: usize = per_frame.iter().map(|s| s.lit).sum();
    let blanked: usize = per_frame.iter().map(|s| s.blanked).sum();
    let jumps: usize = per_frame.iter().map(|s| s.jumps).sum();

    println!("\nexported {take} frames from frame {skip}");
    println!("  segments/frame  {segments:?}");
    println!(
        "  moves           {lit} lit, {blanked} blanked, {jumps} centres",
    );
    println!(
        "  drawn extent    x {lo_x}..{hi_x}, y {lo_y}..{hi_y}  (full scale {FULL_SCALE_X} by {FULL_SCALE_Y})"
    );
    println!("  colours         {colours:?}");
    println!("  intensities     {intensities:?}");
    println!(
        "  ticks/frame     {:?}",
        per_frame.iter().map(|s| s.ticks).collect::<Vec<_>>()
    );
    println!(
        "  busiest frame   {busiest} ticks = {:.3} ms of the {:.3} ms period ({:.0}%)",
        busy_seconds * 1000.0,
        frame_seconds * 1000.0,
        100.0 * busy_seconds / frame_seconds
    );
    println!(
        "  wrote           {} samples, {:.3} s, to {out}\n",
        trace.len(),
        trace.last_t().unwrap_or(0.0)
    );

    assert!(trace.len() > 0, "the export drew nothing");
    assert!(lit > 0, "no lit strokes were exported");
    assert!(blanked > 0, "blanked travel is missing — the beam teleports");
    assert!(
        busy_seconds <= frame_seconds,
        "at {TICKS_PER_SECOND} ticks/s the busiest frame takes {busy_seconds:.4} s, \
         longer than the {frame_seconds:.4} s it has"
    );
}

/// A lit move's drive: its colour's channels, scaled by its intensity.
fn beam_drive(mv: &BeamMove) -> [f32; 3] {
    let level = intensity_drive(mv.intensity);
    colour_rgb(mv.color).map(|c| c * level)
}

/// Push a sample, failing the test rather than writing a trace the renderer
/// would refuse.
fn push(trace: &mut Trace, x: f32, y: f32, drive: [f32; 3], t: f32, flags: u32) {
    trace
        .push(x, y, drive, t, flags)
        .unwrap_or_else(|e| panic!("sample {}: {e}", trace.len()));
}
