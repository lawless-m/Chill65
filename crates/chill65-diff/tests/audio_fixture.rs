//! The audio path, against the verilated core, with the game removed.
//!
//! ```text
//! cargo test -p chill65-diff --test audio_fixture -- --ignored --nocapture
//! ```
//!
//! # Why a fixture and not a trace
//!
//! The same reason `mob_fixture.rs` uses one: our runtime and an oracle are not
//! in the same game state during a recording, so the sound they make during one
//! is not comparable. `fixture/audio.MAC` removes the game — it programs both
//! chips to known values, strobes STIMER and idles, so whatever comes out is a
//! function of the audio hardware alone.
//!
//! # What this establishes that the unit tests do not
//!
//! The path was built by measuring one behaviour at a time: the polynomials,
//! the two clock divisors, the eight distortion settings. Each of those looked
//! at a single signal in isolation, and `pokey.rs`'s tests assert those
//! measurements. This compares the whole path **sample for sample against the
//! hardware**, which is a different question — parts that are individually
//! right can still be wired together wrongly, and only this notices.
//!
//! # Why the core and not MAME
//!
//! The simulation reads its seven devices as a flat file and audits none of
//! them, so it boots a program of ours; MAME checks its set against its own
//! CRC-32s and will not. No game bytes take part in this test at all.

use std::path::PathBuf;
use std::process::Command;

use chill65_diff::images::workspace_root;
use chill65_diff::mister;
use chill65_diff::trace::Trace;
use chill65_runtime::frame::run_frame;
use chill65_runtime::{Cpu, Machine};

const FRAMES: u32 = 12;

/// Compare from six frames in, by which point both sides are long past their
/// power-on sequences and the programme has been idling for millions of cycles.
const WINDOW_START: usize = 6 * 20_480;
/// Two frames of samples — 40,000 consecutive bytes that must match exactly.
const WINDOW: usize = 40_000;

/// How far apart the two sides' cycle origins are allowed to be.
///
/// They cannot share one: our cycle 0 is the CPU's first cycle after reset,
/// while the core's audio recording begins the tick after `reset_n` is
/// released, with the core's own start-up in between. That is a small constant,
/// not a free parameter — the bound is asserted, and the lag is printed so a
/// change in it is visible rather than absorbed.
const MAX_LAG: usize = 8_192;

/// Assemble `fixture/audio.MAC` through the real assembler CLI.
fn assemble() -> Vec<u8> {
    let root = workspace_root();
    let out = root.join("target/audio-fixture/prog.bin");
    std::fs::create_dir_all(out.parent().expect("parent")).expect("out dir");
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["run", "-q", "-p", "chill65-asm", "--", "audio.MAC"])
        .arg("-I")
        .arg(root.join("crates/chill65-diff/fixture"))
        .arg("-o")
        .arg(&out)
        .args(["--base", "A000", "--end", "FFFF"])
        .status()
        .expect("run chill65-asm");
    assert!(status.success(), "the fixture failed to assemble");
    std::fs::read(&out).expect("fixture image")
}

/// Run the fixture on our runtime and return its SOUT stream, one byte per CPU
/// cycle, concatenated across frames.
fn play(prog: &[u8]) -> Vec<u8> {
    let mut m = Machine::new();
    m.load_roms(prog, &vec![0u8; 0x4000]).expect("load");
    m.set_audio_enabled(true);
    let mut cpu = Cpu::new();
    cpu.reset(&mut m);
    let mut out = Vec::with_capacity(FRAMES as usize * 20_480);
    for _ in 0..FRAMES {
        run_frame(&mut cpu, &mut m).expect("frame");
        out.extend_from_slice(m.audio_samples());
    }
    out
}

/// The lag at which `theirs` reproduces `ours` over the window, if any.
///
/// Candidates are screened on a short prefix before the whole window is
/// checked, so the search is fast without being any less strict: the returned
/// lag has matched all [`WINDOW`] bytes.
fn align(ours: &[u8], theirs: &[u8]) -> Option<usize> {
    let want = ours.get(WINDOW_START..WINDOW_START + WINDOW)?;
    (0..MAX_LAG).find(|&lag| {
        let at = WINDOW_START + lag;
        match theirs.get(at..at + WINDOW) {
            Some(got) => got[..256] == want[..256] && got == want,
            None => false,
        }
    })
}

/// Where two streams first differ at a given lag, for reporting a failure
/// precisely rather than as a count.
fn first_difference(ours: &[u8], theirs: &[u8], lag: usize) -> Option<(usize, u8, u8)> {
    (WINDOW_START..WINDOW_START + WINDOW).find_map(|i| {
        let b = *theirs.get(i + lag)?;
        (ours[i] != b).then_some((i, ours[i], b))
    })
}

/// Ignored rather than part of a plain `cargo test`, because it builds the
/// verilator simulation — minutes of compilation and a tool not everyone has.
#[test]
#[ignore = "needs verilator on PATH"]
fn the_audio_path_agrees_with_the_verilated_core() {
    if !mister::have_verilator() {
        eprintln!("verilator not on PATH — skipping");
        return;
    }
    let prog = assemble();

    // The download blob, in the core's own device order: 1f, 1h, 8d, 8b, 1k,
    // 1l, 1n. The castle-data bank and the picture ROMs are unused and zeroed —
    // this fixture draws nothing.
    let mut blob = vec![0u8; 0x8000];
    blob.extend_from_slice(&prog);
    let path: PathBuf = workspace_root().join("target/mister-sim/audio-fixture.bin");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("sim dir");
    std::fs::write(&path, &blob).expect("write blob");

    let capture = mister::capture_blob(&path, &Trace::idle(FRAMES as usize), FRAMES)
        .expect("the core should boot our program");
    let theirs = capture.audio;

    let ours = play(&prog);
    eprintln!(
        "audio fixture: {} samples ours, {} theirs; comparing {WINDOW} from cycle {WINDOW_START}",
        ours.len(),
        theirs.len()
    );

    // The fixture must actually be making a sound, or every assertion below
    // passes on two streams of zeroes.
    let distinct = {
        let mut v: Vec<u8> = ours[WINDOW_START..WINDOW_START + WINDOW].to_vec();
        v.sort_unstable();
        v.dedup();
        v
    };
    eprintln!("  levels present in our window: {distinct:?}");
    assert!(
        distinct.len() > 2,
        "the fixture produced {} distinct levels — it is proving nothing",
        distinct.len()
    );
    assert!(
        !distinct.contains(&0),
        "the volume-only channel should hold every sample off zero"
    );

    let lag = align(&ours, &theirs).unwrap_or_else(|| {
        // Report precisely rather than loosening: say which cycle and which
        // values, at the lag that got furthest.
        let best = (0..MAX_LAG)
            .max_by_key(|&lag| {
                (WINDOW_START..WINDOW_START + WINDOW)
                    .take_while(|&i| theirs.get(i + lag) == Some(&ours[i]))
                    .count()
            })
            .expect("a lag");
        let matched = (WINDOW_START..WINDOW_START + WINDOW)
            .take_while(|&i| theirs.get(i + best) == Some(&ours[i]))
            .count();
        match first_difference(&ours, &theirs, best) {
            Some((i, a, b)) => panic!(
                "no lag within {MAX_LAG} aligns the streams. Best lag {best} matched \
                 {matched} samples, then cycle {i}: ours {a}, theirs {b}"
            ),
            None => panic!("no lag within {MAX_LAG} aligns the streams (best {best})"),
        }
    });

    eprintln!("  aligned at lag {lag}, {WINDOW} consecutive samples identical");
    assert!(
        lag < MAX_LAG,
        "the origins are {lag} cycles apart, further than a start-up difference explains"
    );

    // The control. A gate that cannot fail is not a gate: change one AUDF byte
    // in our own programme and the same comparison must stop finding any lag.
    let control = detune(&prog);
    let ours_detuned = play(&control);
    assert_ne!(ours, ours_detuned, "the control did not change the sound");
    let control_lag = align(&ours_detuned, &theirs);
    eprintln!("  control (AUDF0 5 -> 6): aligned at {control_lag:?}");
    assert!(
        control_lag.is_none(),
        "a detuned fixture still matched the core at lag {control_lag:?}, so the \
         comparison is not measuring the audio path at all"
    );
}

/// Change POKEY0's channel-1 frequency from 5 to 6, by patching the immediate
/// in our own programme.
///
/// The pattern is `LDA #$05 / STA $9800`, which the fixture contains exactly
/// once; the assertion is what keeps this honest if the fixture is edited.
fn detune(prog: &[u8]) -> Vec<u8> {
    const PATTERN: [u8; 5] = [0xA9, 0x05, 0x8D, 0x00, 0x98];
    let hits: Vec<usize> = prog
        .windows(PATTERN.len())
        .enumerate()
        .filter(|(_, w)| *w == PATTERN)
        .map(|(i, _)| i)
        .collect();
    assert_eq!(hits.len(), 1, "expected one `LDA #$05 / STA $9800` in the fixture");
    let mut out = prog.to_vec();
    out[hits[0] + 1] = 0x06;
    out
}
