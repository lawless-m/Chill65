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
//! That hashes *colour RAM addresses*, which only exist inside our machine
//! model — MAME hands out finished pixels and the RTL emits RGB directly off
//! the video bus, and neither can say which colour RAM entry a pixel came from.
//! So the harness hashes the canonical RGB instead, with the same FNV-1a the
//! runtime already uses.

use chill65_runtime::video::{cram_rgb, fnv1a, HEIGHT, WIDTH};

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
    /// When this frame was captured, in emulated CPU cycles since power-on, or
    /// `None` from a reference that cannot say.
    ///
    /// **The point of this field.** The harness compares frame *N* against
    /// frame *N* and reports the first index that differs, which silently
    /// assumes the two are the same frame. Nothing established that, and §12 of
    /// `harness.md` shows the assumption does not hold: 161 frames of apparent
    /// agreement were 161 frames in which both sides were blank, and the
    /// oracle's picture gains content a frame or two after ours. A stamp on
    /// every frame turns the offset from an assumption into a measurement.
    ///
    /// Units are CPU cycles at 1.25 MHz — 20,480 to a frame — because that is
    /// the one quantity all three implementations can express. It is *emulated
    /// time*, not instructions retired.
    pub cycles: Option<u64>,
}

impl Frame {
    /// Hash `rgb`, retaining the bytes only if `keep`.
    ///
    /// Unstamped: a reference that knows its capture instant adds it with
    /// [`Frame::at`].
    pub fn new(rgb: Vec<u8>, keep: bool) -> Frame {
        Frame {
            hash: fnv1a(&rgb),
            rgb: if keep { Some(rgb) } else { None },
            cycles: None,
        }
    }

    /// Stamp the frame with the emulated cycle count at which it was captured.
    ///
    /// Never folded into [`Frame::hash`], which stays FNV-1a over the RGB bytes
    /// and nothing else — two implementations agreeing about a picture must go
    /// on agreeing whether or not they can date it.
    pub fn at(mut self, cycles: u64) -> Frame {
        self.cycles = Some(cycles);
        self
    }
}

/// Expand our machine's per-pixel colour RAM addresses into canonical RGB.
///
/// `Machine::framebuffer` already arbitrated bitmap against motion objects and
/// returned the five-bit colour RAM address, so this only looks the colour up —
/// the same thing `write_ppm` in `chill65-runtime/src/bin/ccrun.rs` does. It
/// must **not** add [`BITMAP_CRAM_BASE`] itself: that offset is inside the
/// address now, and adding it twice would shift every pixel by sixteen.
pub fn rgb_from_indices(picture: &[u8], cram: &[u16; 32]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(picture.len() * 3);
    for &pixel in picture {
        let entry = cram[pixel as usize & 0x1F];
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
    use chill65_runtime::video::BITMAP_CRAM_BASE;

    /// Colour RAM where entry `BITMAP_CRAM_BASE + i` is distinguishable.
    fn ramp_cram() -> [u16; 32] {
        let mut cram = [0u16; 32];
        for i in 0..32 {
            cram[i] = (i as u16) * 9;
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
    fn addresses_go_through_colour_ram_unshifted() {
        let cram = ramp_cram();
        // A bitmap pixel of nibble 0 arrives as address BITMAP_CRAM_BASE, and
        // a motion-object pixel can arrive as anything from 0 to 15 — so the
        // address is used as given, with no offset added.
        let rgb = rgb_from_indices(&[BITMAP_CRAM_BASE as u8, 3], &cram);
        let (r0, g0, b0) = cram_rgb(cram[BITMAP_CRAM_BASE]);
        let (r1, g1, b1) = cram_rgb(cram[3]);
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

    #[test]
    fn a_stamp_does_not_disturb_the_hash() {
        // Frames are compared on the picture. Dating one must not change what
        // it is, or an oracle that can say when a frame was captured would stop
        // agreeing with one that cannot.
        let rgb = vec![4u8, 5, 6];
        let plain = Frame::new(rgb.clone(), false);
        let stamped = Frame::new(rgb, false).at(20_480);
        assert_eq!(plain.hash, stamped.hash);
        assert_eq!(plain.cycles, None);
        assert_eq!(stamped.cycles, Some(20_480));
    }
}
