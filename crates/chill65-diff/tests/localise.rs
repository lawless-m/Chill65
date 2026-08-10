//! Fault injection: the harness must localise a fault it was not told about.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-diff --test localise -- --ignored --nocapture
//! ```
//!
//! # Why this is the honest gate for Phase 3
//!
//! The harness cannot be gated on "our runtime agrees with MAME", because it
//! does not and should not: `gate2.md` lists six hardware claims still marked
//! UNVERIFIED and motion objects are not modelled at all. Divergence against an
//! external oracle is the *output*, not a failure.
//!
//! So the thing to prove is that the machinery works — that when a difference
//! is deliberately introduced in a known place, the harness finds the right
//! frame and names the right routine, and says so the same way twice. The
//! precedent is `chill65-runtime/tests/selftest.rs`, which proves the ROM
//! self-test *fails* on a corrupted byte rather than merely that it passes.
//!
//! # The fixture, and why it is a pair of flips
//!
//! The obvious fixture — flip one bit in a drawing routine — does not test what
//! it appears to. The game checksums its own ROMs at power-on, so *any* altered
//! program byte is caught, and every such fault produces the same picture: the
//! self-test's failure screen. Measured, that is a divergence at frame 371 with
//! 56,836 of 59,392 pixels differing, all attributed to `BITEST`. The flipped
//! byte's own routine never gets a chance to draw anything wrong.
//!
//! The checksum is a longitudinal parity — `EOR NY,TEMP1` in `CST.MAC:337` —
//! so flipping the same bit in *two* bytes of the same 8K device leaves it
//! unchanged. This fixture flips bit 0 at `A408` and `A418`, both inside
//! `LN.F1`, both in device `136022-303.1k`:
//!
//! - `A418` is the `32` of `LDA $32`, the zero-page byte holding the X
//!   coordinate. Flipped, the routine reads `$33` — the Y coordinate — and
//!   draws in the wrong place. This is the fault.
//! - `A408` is the `00` of `LDA #$00` at `LN.F1+1`, whose value is then written
//!   to the OUT1 latch. That latch samples **D3 only** (`CCastles.v:332`, and
//!   see `machine.rs`), and bit 3 of `00` and `01` is the same, so this flip
//!   changes nothing the machine can observe. It exists purely to restore the
//!   parity.
//!
//! One flip does the damage, the other is invisible, and together they satisfy
//! the game's own checksum.

use chill65_diff::images::{build_images, corpus};
use chill65_diff::localise::localise;
use chill65_diff::symbols::Symbols;
use chill65_diff::trace::Trace;
use chill65_diff::OurRuntime;

const FRAMES: u32 = 600;

/// Bit 0 of each of these is flipped together. See the module docs.
const FAULT_SITES: [u16; 2] = [0xA408, 0xA418];

/// The routine both sites fall inside, from `target/boot-artefacts/prog.sym`:
/// `A407 LN.F1 [CRF.MAC]`.
const FAULT_ROUTINE: &str = "LN.F1 [CRF.MAC]";

fn inject(prog: &[u8]) -> Vec<u8> {
    let mut faulty = prog.to_vec();
    for site in FAULT_SITES {
        faulty[site as usize - 0xA000] ^= 0x01;
    }
    faulty
}

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn an_injected_fault_is_localised_to_its_frame_and_its_routine() {
    let Some(corpus) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    let images = build_images(&corpus).expect("build images");
    let symbols = Symbols::load(&images.sym).expect("symbols");
    assert!(!symbols.is_empty(), "the symbol table is empty");
    let trace = Trace::idle(FRAMES as usize);

    let clean = || OurRuntime::new(images.prog.clone(), images.data.clone());
    let faulty = || OurRuntime::new(inject(&images.prog), images.data.clone());

    // (a) The same images against themselves must report nothing. If this ever
    // fails, every other result here is noise.
    let quiet = localise(&mut clean(), &mut clean(), &trace, FRAMES, &symbols)
        .expect("clean run");
    eprintln!("clean vs clean: {quiet}");
    assert_eq!(
        quiet.comparison.first_divergence, None,
        "two identical runs disagreed"
    );
    assert!(quiet.blame.is_empty());

    // (b) The fault is found, after the self-test rather than during it.
    let report = localise(&mut clean(), &mut faulty(), &trace, FRAMES, &symbols)
        .expect("faulty run — the machine must survive the fault");
    eprintln!("clean vs faulty: {report}");

    let frame = report
        .comparison
        .first_divergence
        .expect("the injected fault produced no divergence at all");
    assert!(frame > 0, "diverged at frame 0, before the fault can have run");
    assert_eq!(
        report.comparison.lengths, None,
        "both sides must produce {FRAMES} frames"
    );

    // The self-test's failure screen repaints nearly the whole display. A
    // handful of differing pixels is the signature of a machine still running
    // normally and merely drawing one thing wrongly.
    assert!(
        report.differing > 0 && report.differing < 1000,
        "{} pixels differ — that is a crash or a self-test failure, not a \
         drawing fault",
        report.differing
    );

    // (d) The routine holding the flipped byte is the one blamed.
    let top = report.blame.first().expect("a divergence with no blame");
    assert_eq!(
        top.symbol, FAULT_ROUTINE,
        "top blame is {} but the fault was injected into {FAULT_ROUTINE}",
        top.symbol
    );

    // (c) And the same answer arrives twice.
    let again = localise(&mut clean(), &mut faulty(), &trace, FRAMES, &symbols)
        .expect("second faulty run");
    assert_eq!(
        again.comparison.first_divergence,
        Some(frame),
        "the divergence frame moved between runs"
    );
    assert_eq!(
        again.blame.first().map(|b| &b.symbol),
        Some(&top.symbol),
        "the top-blamed routine moved between runs"
    );
    assert_eq!(again.differing, report.differing);
}
