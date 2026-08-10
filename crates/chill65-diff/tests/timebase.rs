//! Do our frame numbers and an oracle's mean the same frames?
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles CHILL65_MAME=$(which mame) \
//!   cargo test -p chill65-diff --test timebase -- --ignored --nocapture
//! ```
//!
//! # Why this exists
//!
//! The harness compares frame *N* against frame *N* and reports the first index
//! that differs. Nothing established that the two are the same frame, and
//! `harness.md` §12 shows the assumption does not hold: the 161 frames of
//! apparent agreement before the first reported divergence were 161 frames in
//! which **both sides were blank**, and a shift search over the region where
//! there is content found no clean offset — 154, 154, 157, 158, 161, 163, 163,
//! 159, 157 exact matches at shifts −4..+4, which is noise.
//!
//! Every frame now carries the emulated CPU cycle count it was captured at, so
//! the offset is a measurement rather than a search. This test takes it.
//!
//! # What it asserts, and what it only reports
//!
//! Asserted: stamps are strictly monotonic on every side, and our period and
//! the core's are 20,480 cycles. Those are arithmetic, and if they fail the
//! stamps mean nothing.
//!
//! Reported, not asserted: MAME's epoch, and the offsets themselves. MAME's
//! clock starts when the machine starts and ours starts at reset, so the two
//! need not share an origin, and asserting a number before seeing it would be
//! writing down the answer we hoped for.
//!
//! **No pixel equality is asserted anywhere**, and the frames from ~162 to ~208
//! are not used as evidence of anything: that region is the power-on RAM test's
//! cursor being sampled at different phases, which cannot be aligned away
//! (§12).

use chill65_diff::images::{build_images, corpus};
use chill65_diff::mame::MameReference;
use chill65_diff::mister::{self, MisterReference};
use chill65_diff::reference::{Frame, Reference};
use chill65_diff::trace::Trace;
use chill65_diff::OurRuntime;

/// The frame period every side should agree on: 1.25 MHz over 61.035 Hz.
const PERIOD: u64 = chill65_runtime::frame::CYCLES_PER_FRAME as u64;

/// Enough to reach the content transitions §12 records at frames 312 and 335.
const MAME_FRAMES: u32 = 500;
/// The simulation costs about half a second a frame, so this is the shortest
/// run that still passes those transitions.
const MISTER_FRAMES: u32 = 360;

fn stamps(frames: &[Frame], who: &str) -> Vec<u64> {
    frames
        .iter()
        .enumerate()
        .map(|(k, f)| {
            f.cycles
                .unwrap_or_else(|| panic!("{who} frame {k} carries no cycle stamp"))
        })
        .collect()
}

/// Frame period, as the deltas between consecutive stamps.
fn period(stamps: &[u64], who: &str) -> (u64, u64) {
    let deltas: Vec<u64> = stamps.windows(2).map(|w| w[1] - w[0]).collect();
    let lo = *deltas.iter().min().expect("frames");
    let hi = *deltas.iter().max().expect("frames");
    eprintln!("  {who:<8} period {lo}..{hi} cycles (want {PERIOD})");
    (lo, hi)
}

/// The oracle's stamp minus ours, per frame, split into whole frames and the
/// sub-frame remainder.
/// Whole frames and signed sub-frame remainder.
///
/// **Nearest, not floor.** Flooring would call a difference of −2 cycles "one
/// whole frame behind, 20,478 cycles into it", which is arithmetically true and
/// completely misleading — and the comparison downstream turns this number into
/// an applied shift, so a floor would shift by a frame on the strength of two
/// cycles.
fn split(delta: i64) -> (i64, i64) {
    let whole = (delta as f64 / PERIOD as f64).round() as i64;
    (whole, delta - whole * PERIOD as i64)
}

fn offsets(ours: &[u64], theirs: &[u64], who: &str) {
    let n = ours.len().min(theirs.len());
    let mut seen: Vec<i64> = Vec::new();
    for k in 0..n {
        let (whole, _) = split(theirs[k] as i64 - ours[k] as i64);
        if seen.last() != Some(&whole) {
            seen.push(whole);
        }
    }

    let first = theirs[0] as i64 - ours[0] as i64;
    let last = theirs[n - 1] as i64 - ours[n - 1] as i64;
    let (whole, phase) = split(first);
    eprintln!(
        "  {who:<8} stamp difference: {first:+} cycles at frame 0, {last:+} at frame {}",
        n - 1
    );
    eprintln!(
        "  {who:<8} whole frames {whole:+}, sub-frame phase {phase:+} cycles{}",
        if first == last { ", constant" } else { "" }
    );
    if first != last {
        // Our own frames overshoot their deadline by a fraction of a cycle on
        // average, and the excess accumulates; the oracles' clocks are exact.
        // So a slow negative slide is ours, not a difference of rate.
        eprintln!(
            "  {who:<8} slide over {n} frames: {} cycles ({:.2}/frame)",
            last - first,
            (last - first) as f64 / n as f64
        );
    }
    eprintln!("  {who:<8} distinct whole-frame offsets across the run: {seen:?}");
}

/// The frame at which a side first reaches `want` lit pixels — the observable
/// §12 used, recomputed here so the two can be compared.
fn first_at_least(frames: &[Frame], want: usize) -> Option<usize> {
    frames.iter().position(|f| {
        f.rgb
            .as_ref()
            .map(|rgb| rgb.chunks_exact(3).filter(|p| *p != [0, 0, 0]).count() >= want)
            .unwrap_or(false)
    })
}

#[test]
#[ignore = "needs CHILL65_CORPUS, and CHILL65_MAME or verilator"]
fn the_oracles_are_on_our_timebase() {
    let Some(game) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let images = build_images(&game).expect("images");
    let trace = Trace::idle(MAME_FRAMES as usize);

    // --- our side -------------------------------------------------------
    let mut ours = OurRuntime::new(images.prog.clone(), images.data.clone(), Some(images.mob.clone()));
    let mine = ours.run(&trace, MAME_FRAMES, true).expect("our run");
    let my_stamps = stamps(&mine, "ours");

    eprintln!("\ntimebase, {MAME_FRAMES} frames of idle-attract:");
    let (lo, hi) = period(&my_stamps, "ours");
    assert_eq!(lo, PERIOD, "our shortest frame");
    assert!(
        hi <= PERIOD + 7,
        "our longest frame {hi} exceeds one instruction's overshoot"
    );
    for pair in my_stamps.windows(2) {
        assert!(pair[1] > pair[0], "our stamps must increase");
    }

    // --- MAME -----------------------------------------------------------
    if let Some(mut mame) = MameReference::discover(&game) {
        let theirs = mame.run(&trace, MAME_FRAMES, true).expect("mame run");
        let their_stamps = stamps(&theirs, "mame");
        for pair in their_stamps.windows(2) {
            assert!(pair[1] > pair[0], "MAME's stamps must increase");
        }
        // Reported, not asserted: MAME's clock need not share our origin.
        period(&their_stamps, "mame");
        offsets(&my_stamps, &their_stamps, "mame");

        // The observable §12 measured by eye, now beside the stamps.
        for want in [88usize, 176] {
            eprintln!(
                "  mame     first frame with >={want} lit: ours {:?}, mame {:?}",
                first_at_least(&mine, want),
                first_at_least(&theirs, want)
            );
        }
    } else {
        eprintln!("  CHILL65_MAME unset — skipping the MAME arm");
    }

    // --- the verilated core ---------------------------------------------
    if !mister::have_verilator() {
        eprintln!("  verilator not on PATH — skipping the MiSTer arm");
        return;
    }
    let short = Trace::idle(MISTER_FRAMES as usize);
    let mut ours = OurRuntime::new(images.prog, images.data, Some(images.mob));
    let mine = ours.run(&short, MISTER_FRAMES, true).expect("our short run");
    let my_stamps = stamps(&mine, "ours");

    let mut core = MisterReference::discover(&game).expect("verilator is present");
    let theirs = core.run(&short, MISTER_FRAMES, true).expect("mister run");
    let their_stamps = stamps(&theirs, "mister");
    for pair in their_stamps.windows(2) {
        assert!(pair[1] > pair[0], "the core's stamps must increase");
    }
    let (lo, hi) = period(&their_stamps, "mister");
    assert_eq!((lo, hi), (PERIOD, PERIOD), "the core's frame period");
    offsets(&my_stamps, &their_stamps, "mister");
    for want in [88usize, 176] {
        eprintln!(
            "  mister   first frame with >={want} lit: ours {:?}, mister {:?}",
            first_at_least(&mine, want),
            first_at_least(&theirs, want)
        );
    }
}
