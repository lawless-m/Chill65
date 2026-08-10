//! MAME boots the reconstructed set and hands back frames we can compare.
//!
//! ```text
//! CHILL65_MAME=$(which mame) CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-diff --test mame -- --ignored --nocapture
//! ```
//!
//! Capture only: driving MAME from a trace is a later task. What this pins down
//! is that the oracle starts, that its geometry is ours, and that it is drawing
//! something rather than handing back a black screen.
//!
//! Skips cleanly when either variable is unset. An oracle that is not installed
//! is not a failure.

use chill65_diff::images::corpus;
use chill65_diff::mame::Mame;
use chill65_diff::reference::FRAME_BYTES;

/// Past the point our own runtime first draws, which `boot.rs` measures at
/// frame 162. A shorter capture is all black and proves nothing.
const FRAMES: u32 = 400;

#[test]
#[ignore = "needs CHILL65_MAME and CHILL65_CORPUS"]
fn mame_boots_the_rebuilt_set_and_draws() {
    let Some(mame) = Mame::discover() else {
        eprintln!("CHILL65_MAME unset — skipping");
        return;
    };
    let Some(game) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };
    eprintln!("mame: {}", mame.version().expect("mame -version"));

    let frames = mame.capture(&game, FRAMES).expect("capture");
    assert_eq!(frames.len(), FRAMES as usize);
    for (k, f) in frames.iter().enumerate() {
        assert_eq!(f.len(), FRAME_BYTES, "frame {k} is the wrong size");
    }

    // Past power-on it must be drawing. A capture of nothing but black would
    // satisfy every other assertion here.
    let lit: Vec<usize> = frames
        .iter()
        .map(|f| f.chunks_exact(3).filter(|p| p != &[0, 0, 0]).count())
        .collect();
    let best = lit.iter().copied().max().expect("frames captured");
    eprintln!(
        "{FRAMES} frames of {FRAME_BYTES} bytes; lit pixels: first {}, last {}, most {best}",
        lit[0],
        lit[FRAMES as usize - 1]
    );
    assert!(
        best > 0,
        "every captured frame was entirely black — MAME booted but drew nothing"
    );

    // And it is not handing back the same still image every time.
    let distinct: std::collections::HashSet<&Vec<u8>> = frames.iter().collect();
    eprintln!("{} distinct images across {FRAMES} frames", distinct.len());
    assert!(
        distinct.len() > 1,
        "all {FRAMES} frames were identical — the capture is stuck"
    );
}
