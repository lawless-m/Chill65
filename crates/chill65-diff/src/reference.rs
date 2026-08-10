//! The oracle seam: what every reference implementation must offer.
//!
//! Our runtime, MAME and a verilated MiSTer core have nothing in common
//! internally, so the harness only ever asks them for one thing — the picture,
//! frame by frame, given the same input. [`Reference`] is that request.
//!
//! # The canonical frame
//!
//! Comparison happens on **256x232 RGB24, row-major**, three bytes per pixel,
//! [`FRAME_BYTES`] in all. That is the one form every implementation can be
//! made to produce; anything narrower would privilege ours.
//!
//! This is why the harness does not reuse [`chill65_runtime::Machine::frame_hash`].
//! That hashes *palette indices*, which only exist inside our machine model —
//! MAME hands out finished pixels and the RTL emits RGB directly off the video
//! bus, and neither can say which colour RAM entry a pixel came from. So the
//! harness hashes the canonical RGB instead, with the same FNV-1a the runtime
//! already uses.

use chill65_runtime::video::{cram_rgb, fnv1a, BITMAP_CRAM_BASE, HEIGHT, WIDTH};

use crate::trace::Trace;

/// Size of one canonical frame: 256 x 232 x 3.
pub const FRAME_BYTES: usize = WIDTH * HEIGHT * 3;

/// One frame's output from a reference implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// FNV-1a over the canonical RGB bytes. Always present — it is the cheap
    /// thing a whole-run comparison needs.
    pub hash: u64,
    /// The canonical RGB bytes, kept only when asked for. Six hundred frames of
    /// pixels is 107 MB, so a divergence *search* keeps hashes alone and only
    /// the frames around a divergence are re-run for pixels.
    pub rgb: Option<Vec<u8>>,
}

impl Frame {
    /// Hash `rgb`, retaining the bytes only if `keep`.
    pub fn new(rgb: Vec<u8>, keep: bool) -> Frame {
        Frame {
            hash: fnv1a(&rgb),
            rgb: if keep { Some(rgb) } else { None },
        }
    }
}

/// Expand our machine's palette indices into canonical RGB.
///
/// The mapping is the one `write_ppm` in `chill65-runtime/src/bin/ccrun.rs`
/// applies: index into colour RAM at [`BITMAP_CRAM_BASE`], then [`cram_rgb`].
pub fn rgb_from_indices(picture: &[u8], cram: &[u16; 32]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(picture.len() * 3);
    for &pixel in picture {
        let entry = cram[(BITMAP_CRAM_BASE + pixel as usize) & 0x1F];
        let (r, g, b) = cram_rgb(entry);
        rgb.extend_from_slice(&[r, g, b]);
    }
    rgb
}

/// An implementation the harness can drive and compare.
///
/// Implementors run `frames` frames from a cold start, taking frame `k`'s input
/// from `trace.frames[k]`, or idle input when the trace is shorter than the run.
/// A run must be **deterministic**: the same trace twice must give the same
/// hashes, or nothing built on top of it means anything.
pub trait Reference {
    fn run(
        &mut self,
        trace: &Trace,
        frames: u32,
        want_pixels: bool,
    ) -> Result<Vec<Frame>, String>;

    /// Just the per-frame hashes — what a divergence search compares.
    fn hashes(&mut self, trace: &Trace, frames: u32) -> Result<Vec<u64>, String> {
        Ok(self
            .run(trace, frames, false)?
            .into_iter()
            .map(|f| f.hash)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Colour RAM where entry `BITMAP_CRAM_BASE + i` is distinguishable.
    fn ramp_cram() -> [u16; 32] {
        let mut cram = [0u16; 32];
        for i in 0..16 {
            cram[(BITMAP_CRAM_BASE + i) & 0x1F] = i as u16;
        }
        cram
    }

    #[test]
    fn rgb_is_three_bytes_per_pixel() {
        let picture = vec![0u8; WIDTH * HEIGHT];
        let rgb = rgb_from_indices(&picture, &ramp_cram());
        assert_eq!(rgb.len(), FRAME_BYTES);
    }

    #[test]
    fn indices_go_through_colour_ram() {
        let cram = ramp_cram();
        let rgb = rgb_from_indices(&[0, 1], &cram);
        let (r0, g0, b0) = cram_rgb(cram[BITMAP_CRAM_BASE]);
        let (r1, g1, b1) = cram_rgb(cram[BITMAP_CRAM_BASE + 1]);
        assert_eq!(rgb, vec![r0, g0, b0, r1, g1, b1]);
    }

    #[test]
    fn one_changed_pixel_changes_the_hash() {
        let cram = ramp_cram();
        let mut picture = vec![0u8; WIDTH * HEIGHT];
        let before = Frame::new(rgb_from_indices(&picture, &cram), false);
        picture[WIDTH * HEIGHT - 1] = 9;
        let after = Frame::new(rgb_from_indices(&picture, &cram), false);
        assert_ne!(before.hash, after.hash);
    }

    #[test]
    fn pixels_are_kept_only_when_asked() {
        let rgb = vec![1u8, 2, 3];
        assert_eq!(Frame::new(rgb.clone(), true).rgb, Some(rgb.clone()));
        assert_eq!(Frame::new(rgb.clone(), false).rgb, None);
        // The hash does not depend on whether the bytes were retained.
        assert_eq!(
            Frame::new(rgb.clone(), true).hash,
            Frame::new(rgb, false).hash
        );
    }
}
