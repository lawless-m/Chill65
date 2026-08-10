//! MAME as an external oracle.
//!
//! Boots the reconstructed `ccastles3` set headless and captures every frame,
//! through MAME's own Lua interface. Nothing here modifies MAME or depends on
//! its internals beyond the documented scripting API.
//!
//! # Gating
//!
//! Everything is gated on `CHILL65_MAME`, the path to the `mame` executable.
//! When it is unset, callers **skip cleanly** — the same convention
//! `CHILL65_CORPUS` follows. An oracle that is not installed is not a failure.
//!
//! # Geometry
//!
//! MAME reports `:screen` as **256 x 232** at 61.035 Hz, which is exactly the
//! harness's canonical frame — no crop, no scale, no resampling. The capture
//! script writes the dimensions it actually saw to a sidecar file and the Rust
//! side checks them, so a future MAME that changes the geometry produces an
//! error rather than a silently misaligned picture.
//!
//! # Pixel format
//!
//! `screen:pixels()` returns the screen bitmap as packed 32-bit values, four
//! bytes each, little-endian on this host: `B G R A`. The alpha byte is
//! discarded.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::images::workspace_root;
use crate::reference::{Frame, Reference, FRAME_BYTES};
use crate::romset::{build_and_write, zip_path};
use crate::trace::{FrameInput, Trace, ALL_SWITCHES};

/// Environment variable naming the MAME executable.
pub const MAME_ENV: &str = "CHILL65_MAME";

/// The MAME set this harness drives. See `romset` for why it is not the parent.
pub const SET: &str = "ccastles3";

/// The capture script, committed alongside this module.
const CAPTURE_LUA: &str = include_str!("../lua/capture.lua");

/// The `mame` executable, if the environment names one.
pub fn mame_path() -> Option<PathBuf> {
    std::env::var(MAME_ENV).ok().map(PathBuf::from)
}

/// A MAME installation the harness can drive.
pub struct Mame {
    exe: PathBuf,
    /// Scratch root under `target/`, holding the script, raw frames and the
    /// throwaway cfg/nvram directories.
    scratch: PathBuf,
}

impl Mame {
    /// Locate MAME, or `None` if `CHILL65_MAME` is unset.
    pub fn discover() -> Option<Mame> {
        let exe = mame_path()?;
        Some(Mame {
            exe,
            scratch: workspace_root().join("target/boot-artefacts/mame"),
        })
    }

    /// MAME's reported version, which also proves the executable runs.
    pub fn version(&self) -> Result<String, String> {
        let out = Command::new(&self.exe)
            .arg("-version")
            .output()
            .map_err(|e| format!("{}: {e}", self.exe.display()))?;
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Rebuild the ROM set if needed and return the directory holding it.
    fn rompath(&self, corpus: &Path) -> Result<PathBuf, String> {
        let zip = zip_path();
        if !zip.exists() {
            let (set, _) = build_and_write(corpus)?;
            if !set.all_ok() {
                return Err("the rebuilt ROM set does not match MAME's CRCs".into());
            }
        }
        Ok(zip
            .parent()
            .expect("the archive has a parent directory")
            .to_path_buf())
    }

    /// Boot the set with no input at all and capture `frames` frames.
    pub fn capture(&self, corpus: &Path, frames: u32) -> Result<Vec<Vec<u8>>, String> {
        self.run_trace(corpus, &Trace::idle(frames as usize), frames)
    }

    /// Boot the set, drive it from `trace`, and capture `frames` frames as
    /// canonical RGB24.
    pub fn run_trace(
        &self,
        corpus: &Path,
        trace: &Trace,
        frames: u32,
    ) -> Result<Vec<Vec<u8>>, String> {
        let rompath = self.rompath(corpus)?;

        // Fresh scratch every run: cfg and nvram persisting between runs is the
        // obvious way for one run to influence the next, and determinism is not
        // negotiable for an oracle.
        let _ = std::fs::remove_dir_all(&self.scratch);
        let cfg = self.scratch.join("cfg");
        let nvram = self.scratch.join("nvram");
        for dir in [&self.scratch, &cfg, &nvram] {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }

        let script = self.scratch.join("capture.lua");
        std::fs::write(&script, CAPTURE_LUA).map_err(|e| format!("{}: {e}", script.display()))?;
        let raw = self.scratch.join("frames.raw");

        let input = self.scratch.join("input.txt");
        std::fs::write(&input, input_file(trace, frames))
            .map_err(|e| format!("{}: {e}", input.display()))?;

        // A generous ceiling: the script exits as soon as it has its frames, so
        // this only bounds a run that never gets there.
        let seconds = (frames / 60 + 10).to_string();

        let out = Command::new(&self.exe)
            .arg(SET)
            .args(["-rompath".as_ref(), rompath.as_os_str()])
            .args(["-video", "none", "-sound", "none", "-nothrottle"])
            .args(["-seconds_to_run", &seconds])
            .args(["-autoboot_script".as_ref(), script.as_os_str()])
            .args(["-autoboot_delay", "0"])
            .args(["-cfg_directory".as_ref(), cfg.as_os_str()])
            .args(["-nvram_directory".as_ref(), nvram.as_os_str()])
            .env("CHILL65_MAME_OUT", &raw)
            .env("CHILL65_MAME_FRAMES", frames.to_string())
            .env("CHILL65_MAME_INPUT", &input)
            .output()
            .map_err(|e| format!("running {}: {e}", self.exe.display()))?;

        if !raw.exists() {
            return Err(format!(
                "MAME wrote no frames.\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&out.stdout).trim(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }

        self.check_dims(&raw)?;
        let blob = std::fs::read(&raw).map_err(|e| format!("{}: {e}", raw.display()))?;
        decode(&blob, frames)
    }

    /// The capture script records what geometry it saw; insist it is ours.
    fn check_dims(&self, raw: &Path) -> Result<(), String> {
        let path = raw.with_extension("raw.dims");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut fields = text.split_whitespace();
        let w: usize = fields
            .next()
            .and_then(|f| f.parse().ok())
            .ok_or("no width in the dims sidecar")?;
        let h: usize = fields
            .next()
            .and_then(|f| f.parse().ok())
            .ok_or("no height in the dims sidecar")?;
        if (w, h) != (chill65_runtime::video::WIDTH, chill65_runtime::video::HEIGHT) {
            return Err(format!(
                "MAME's screen is {w}x{h}, the harness canonical frame is {}x{}. \
                 Implement an explicit crop or mapping — never scale silently.",
                chill65_runtime::video::WIDTH,
                chill65_runtime::video::HEIGHT
            ));
        }
        Ok(())
    }
}

/// Serialise a trace as the per-frame input file the capture script reads:
/// one line of `<IN0 mask> <trackball X> <trackball Y>` per frame.
///
/// # The two translations this performs
///
/// **Switches become an `IN0` mask.** MAME's `:IN0` fields sit on exactly the
/// bits [`chill65_runtime::Switch::bit`] names — `0x40` Start1, `0x80` Start2, `0x10` Service
/// Mode, `0x08` Tilt, `0x04` Service 1, `0x02` Coin 1, `0x01` Coin 2 — which is
/// an independent confirmation of the switch model in `chill65-runtime`.
///
/// **Deltas become positions.** Our traces record per-frame movement, but the
/// LETA reports an *absolute* count that the game differences itself
/// (`CEN.MAC:317`, `TR.DEL`). So the deltas are accumulated here and the
/// running total, taken modulo 256, is what MAME is told. One trace unit is one
/// CPU-visible count: `input.rs` marks the host-delta-to-count scaling
/// UNVERIFIED, and this is that choice made explicit rather than buried.
pub fn input_file(trace: &Trace, frames: u32) -> String {
    let mut out = String::with_capacity(frames as usize * 12);
    let (mut x, mut y) = (0i32, 0i32);
    for k in 0..frames as usize {
        let input = trace.frames.get(k).copied().unwrap_or_else(FrameInput::idle);
        x = x.wrapping_add(input.dx);
        y = y.wrapping_add(input.dy);
        let mask: u32 = ALL_SWITCHES
            .into_iter()
            .filter(|&s| input.switches.contains(s))
            .map(|s| 1u32 << s.bit())
            .sum();
        out.push_str(&format!(
            "{mask} {} {}\n",
            x.rem_euclid(256),
            y.rem_euclid(256)
        ));
    }
    out
}

/// Our runtime's counterpart on the other side of the seam.
pub struct MameReference {
    mame: Mame,
    corpus: PathBuf,
}

impl MameReference {
    /// `None` when `CHILL65_MAME` is unset.
    pub fn discover(corpus: impl Into<PathBuf>) -> Option<MameReference> {
        Some(MameReference {
            mame: Mame::discover()?,
            corpus: corpus.into(),
        })
    }
}

impl Reference for MameReference {
    fn run(&mut self, trace: &Trace, frames: u32, want_pixels: bool) -> Result<Vec<Frame>, String> {
        Ok(self
            .mame
            .run_trace(&self.corpus, trace, frames)?
            .into_iter()
            .map(|rgb| Frame::new(rgb, want_pixels))
            .collect())
    }
}

/// Turn MAME's packed 32-bit pixels into canonical RGB24 frames.
pub fn decode(blob: &[u8], frames: u32) -> Result<Vec<Vec<u8>>, String> {
    let pixels = FRAME_BYTES / 3;
    let stride = pixels * 4;
    if blob.len() < stride {
        return Err(format!(
            "captured {} bytes, less than one {stride}-byte frame",
            blob.len()
        ));
    }
    if blob.len() % stride != 0 {
        return Err(format!(
            "captured {} bytes, not a whole number of {stride}-byte frames",
            blob.len()
        ));
    }
    let got = blob.len() / stride;
    if got < frames as usize {
        return Err(format!("captured {got} frames, wanted {frames}"));
    }

    Ok(blob
        .chunks_exact(stride)
        .take(frames as usize)
        .map(|frame| {
            let mut rgb = Vec::with_capacity(FRAME_BYTES);
            for px in frame.chunks_exact(4) {
                // Little-endian ARGB32: B G R A.
                rgb.extend_from_slice(&[px[2], px[1], px[0]]);
            }
            rgb
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packed(pixels: &[(u8, u8, u8)]) -> Vec<u8> {
        pixels
            .iter()
            .flat_map(|&(r, g, b)| [b, g, r, 0xFF])
            .collect()
    }

    #[test]
    fn decodes_argb32_into_rgb24() {
        let mut frame = packed(&[(1, 2, 3), (4, 5, 6)]);
        frame.extend(std::iter::repeat_n(0u8, FRAME_BYTES / 3 * 4 - frame.len()));
        let out = decode(&frame, 1).expect("decode");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].len(), FRAME_BYTES);
        assert_eq!(&out[0][..6], &[1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn splits_a_blob_into_whole_frames() {
        let one = FRAME_BYTES / 3 * 4;
        let blob = vec![0u8; one * 3];
        assert_eq!(decode(&blob, 3).expect("three frames").len(), 3);
        assert_eq!(decode(&blob, 2).expect("take fewer").len(), 2);
    }

    #[test]
    fn a_short_or_ragged_capture_is_an_error_not_a_guess() {
        let one = FRAME_BYTES / 3 * 4;
        assert!(decode(&[0u8; 16], 1).is_err(), "shorter than one frame");
        assert!(decode(&vec![0u8; one + 7], 1).is_err(), "ragged tail");
        assert!(decode(&vec![0u8; one], 2).is_err(), "fewer frames than asked");
    }
}
