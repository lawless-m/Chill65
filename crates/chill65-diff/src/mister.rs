//! The verilated MiSTer core as an external oracle.
//!
//! `Arcade-CrystalCastles_MiSTer` is third-party reference material and is
//! **never modified**. Verilator 5 is stricter than the 4.x era that core was
//! written against, so every accommodation is a flag — see
//! `tools/build-mister-sim.sh`.
//!
//! # Gating
//!
//! Needs `verilator` on `PATH` and `CHILL65_CORPUS` set. Either missing means
//! callers skip cleanly.
//!
//! # The observable window
//!
//! The core's top-level `HBLANK` is `HBLANK2`, the *delayed* blanking — set at
//! hcount 259 and cleared at hcount 7 (`SyncChain.v:35-38`) — and `RGBout` is
//! forced to zero whenever it is asserted (`CCastles.v:353`). So the core emits
//! **252 pixels per line**, not 256: the first columns are blanked at the port
//! and simply cannot be observed from outside.
//!
//! That is a property of the core's interface, not something a flag fixes. The
//! simulation reports the widest active line it saw and the Rust side records
//! it, so a comparison against these frames must confine itself to the columns
//! the core actually emits rather than pretending the rest are black.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::images::workspace_root;
use crate::reference::{Frame, Reference};
use crate::trace::Trace;
use crate::romset::{build_and_write, zip_path};

/// Devices in `dn_addr[15:13]` order, from `ProgramMemory.v` and
/// `MotionObjectPictureRom.v`: 0 ic1F, 1 ic1H, 2 ic8D, 3 ic8B, 4 ic1K, 5 ic1L,
/// 6 ic1N.
const DOWNLOAD_ORDER: [&str; 7] = [
    "136022-101.1f",
    "136022-102.1h",
    "136022-106.8d",
    "136022-107.8b",
    "136022-303.1k",
    "136022-304.1l",
    "136022-305.1n",
];

/// What the simulation reported about the frames it produced.
pub struct Capture {
    /// Canonical 256x232 RGB24 frames, left-aligned, unemitted columns zero.
    pub frames: Vec<Vec<u8>>,
    /// Emulated CPU cycles elapsed at each frame's capture, index aligned with
    /// `frames`.
    ///
    /// The simulation counts master clocks; these are already divided down to
    /// CPU cycles, which is the unit all three implementations share.
    pub cycles: Vec<u64>,
    /// Pixels per line the core actually emitted — 252 for this core.
    pub emitted: usize,
    /// The core's `SOUT` audio output, one 8-bit sample per CPU cycle, from
    /// the first tick after reset is released.
    ///
    /// **The origin is not shared with anything.** The simulation starts
    /// sampling when it releases reset; our runtime starts counting at its own
    /// reset. So a comparison has to establish the constant lag between the two
    /// sequences rather than assume they begin together — the same problem the
    /// frame stamps have, and solved the same way.
    pub audio: Vec<u8>,
}

/// Is verilator available?
pub fn have_verilator() -> bool {
    Command::new("verilator")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn sim_dir() -> PathBuf {
    workspace_root().join("target/mister-sim")
}

/// The vendored core: the RTL every default build reads, and never writes.
pub fn core_dir() -> PathBuf {
    workspace_root().join("Arcade-CrystalCastles_MiSTer")
}

/// Build the simulation, if it is not already built.
pub fn build() -> Result<PathBuf, String> {
    build_at(&core_dir(), &sim_dir())
}

/// Build a simulation from `rtl_parent` into `out`, if it is not already built.
///
/// The pair is explicit so an experiment can verilate a *copy* of the RTL with
/// a change applied and put the two side by side. Only a copy is ever edited —
/// see this module's opening note.
pub fn build_at(rtl_parent: &Path, dir: &Path) -> Result<PathBuf, String> {
    let exe = dir.join("mistersim");
    if exe.exists() {
        return Ok(exe);
    }
    let script = workspace_root().join("tools/build-mister-sim.sh");
    let out = Command::new("sh")
        .arg(&script)
        .arg(rtl_parent)
        .arg(dir)
        .current_dir(workspace_root())
        .output()
        .map_err(|e| format!("{}: {e}", script.display()))?;
    if !out.status.success() {
        return Err(format!(
            "build-mister-sim failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    if !exe.exists() {
        return Err(format!("{} was not produced", exe.display()));
    }
    Ok(exe)
}

/// Write the seven-device blob the core's download port expects.
fn rom_blob(corpus: &Path) -> Result<PathBuf, String> {
    if !zip_path().exists() {
        let (set, _) = build_and_write(corpus)?;
        if !set.all_ok() {
            return Err("the rebuilt ROM set does not match MAME's CRCs".into());
        }
    }
    // Read the members back out of the archive we just verified, rather than
    // rebuilding them by a second route that could drift.
    let (set, _) = build_and_write(corpus)?;
    let _ = &set;

    let dir = sim_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join("roms.bin");

    let mut blob = Vec::with_capacity(DOWNLOAD_ORDER.len() * 0x2000);
    let archive = std::fs::read(zip_path()).map_err(|e| format!("{}: {e}", e))?;
    for name in DOWNLOAD_ORDER {
        blob.extend_from_slice(&member(&archive, name)?);
    }
    std::fs::write(&path, &blob).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Pull one stored member out of the archive our own writer produced.
///
/// Only the `stored` method is ever written, so a member is its bytes verbatim
/// after the local header — no decompression to implement.
fn member(zip: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let mut off = 0usize;
    while off + 30 <= zip.len() && &zip[off..off + 4] == b"PK\x03\x04" {
        let size = u32::from_le_bytes(zip[off + 18..off + 22].try_into().unwrap()) as usize;
        let name_len = u16::from_le_bytes(zip[off + 26..off + 28].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(zip[off + 28..off + 30].try_into().unwrap()) as usize;
        let start = off + 30 + name_len + extra_len;
        let this = std::str::from_utf8(&zip[off + 30..off + 30 + name_len])
            .map_err(|e| format!("member name: {e}"))?;
        if this == name {
            return Ok(zip[start..start + size].to_vec());
        }
        off = start + size;
    }
    Err(format!("{name} is not in the archive"))
}

/// Build if needed, then run the core for `frames` frames with no input.
pub fn capture(corpus: &Path, frames: u32) -> Result<Capture, String> {
    capture_trace(corpus, &Trace::idle(frames as usize), frames)
}

/// Build if needed, then run the core driven from `trace`.
pub fn capture_trace(corpus: &Path, trace: &Trace, frames: u32) -> Result<Capture, String> {
    let roms = rom_blob(corpus)?;
    capture_blob(&roms, trace, frames)
}

/// As [`capture_trace`], but from a ROM blob prepared by the caller.
///
/// The simulation reads its seven devices as a flat file and checks nothing
/// about them, so this will boot **a program of ours** — which is what makes
/// the core usable as an oracle for a fixture. MAME will not: it audits its set
/// against its own CRC-32s, and an original program has none of them.
///
/// The blob is `DOWNLOAD_ORDER` end to end, 8192 bytes each.
pub fn capture_blob(roms: &Path, trace: &Trace, frames: u32) -> Result<Capture, String> {
    build()?;
    capture_blob_in(&sim_dir(), roms, trace, frames)
}

/// As [`capture_blob`], but from a named simulation directory.
///
/// `dir` must already hold a built `mistersim` — [`build_at`] puts one there.
/// The blob is copied in if it lives elsewhere, because the simulation is run
/// with `dir` as its working directory and is given a bare filename.
pub fn capture_blob_in(
    dir: &Path,
    roms: &Path,
    trace: &Trace,
    frames: u32,
) -> Result<Capture, String> {
    let exe = dir.join("mistersim");
    if !exe.exists() {
        return Err(format!("{} has no mistersim", dir.display()));
    }
    let roms = if roms.parent() == Some(dir) {
        roms.to_path_buf()
    } else {
        let dest = dir.join(roms.file_name().ok_or("the blob has no filename")?);
        std::fs::copy(roms, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        dest
    };
    let roms = roms.as_path();
    let dir = dir.to_path_buf();
    let raw = dir.join("frames.raw");

    // The same per-frame `<IN0 mask> <x> <y>` file MAME is given. Both oracles
    // want an absolute trackball position rather than a delta, so one
    // serialiser serves both and the two adapters cannot drift apart.
    let input = dir.join("input.txt");
    std::fs::write(&input, crate::mame::input_file(trace, frames))
        .map_err(|e| format!("{}: {e}", input.display()))?;

    // Run from the simulation directory: ColorMemory.v initialises colour RAM
    // through a relative `$readmem` of "cram.rom", so the working directory
    // decides whether the core powers on with its intended palette.
    let out = Command::new(&exe)
        .current_dir(&dir)
        .args([
            roms.file_name().unwrap().to_str().unwrap(),
            "frames.raw",
            &frames.to_string(),
            "input.txt",
        ])
        .output()
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    if !out.status.success() {
        return Err(format!(
            "mistersim failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let dims = std::fs::read_to_string(dir.join("frames.raw.dims"))
        .map_err(|e| format!("dims sidecar: {e}"))?;
    let fields: Vec<usize> = dims
        .split_whitespace()
        .filter_map(|f| f.parse().ok())
        .collect();
    let [w, h, emitted] = fields[..] else {
        return Err(format!("cannot parse dims {dims:?}"));
    };
    if (w, h) != (chill65_runtime::video::WIDTH, chill65_runtime::video::HEIGHT) {
        return Err(format!("the simulation produced {w}x{h} frames"));
    }

    let blob = std::fs::read(&raw).map_err(|e| format!("{}: {e}", raw.display()))?;
    let stride = w * h * 3;
    if blob.len() < stride * frames as usize {
        return Err(format!(
            "captured {} bytes, want {} frames of {stride}",
            blob.len(),
            frames
        ));
    }
    // Master clocks at each frame write, one per line. The core runs from a
    // 10 MHz master and `Clock.v` divides it by eight for `ce2H`, the CPU clock
    // enable — so eight master clocks is one CPU cycle. That derivation is the
    // same one `chill65-runtime`'s `frame.rs` uses to reach 20,480 cycles a
    // frame, and it is what puts the core on the same timebase as the others.
    let cycles_text = std::fs::read_to_string(dir.join("frames.raw.cycles"))
        .map_err(|e| format!("cycles sidecar: {e}"))?;
    let ticks: Vec<u64> = cycles_text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            l.trim()
                .parse::<u64>()
                .map_err(|e| format!("cycles sidecar: {l:?} is not a tick count: {e}"))
        })
        .collect::<Result<_, _>>()?;
    if ticks.len() != frames as usize {
        return Err(format!(
            "cycles sidecar: {} stamps for {frames} frames",
            ticks.len()
        ));
    }

    // The audio sidecar: raw bytes, one per CPU cycle. Absent or empty is an
    // error rather than silence — a caller asking for audio and getting none
    // would otherwise compare against a sequence that never existed.
    let audio_path = dir.join("frames.raw.audio");
    let audio = std::fs::read(&audio_path)
        .map_err(|e| format!("{}: {e}", audio_path.display()))?;
    if audio.is_empty() {
        return Err(format!("{}: no audio samples", audio_path.display()));
    }

    Ok(Capture {
        frames: blob
            .chunks_exact(stride)
            .take(frames as usize)
            .map(<[u8]>::to_vec)
            .collect(),
        cycles: ticks.iter().map(|t| t / 8).collect(),
        emitted,
        audio,
    })
}

/// The verilated core behind the [`Reference`] seam.
///
/// Runs are cached by trace and frame count. The simulation costs roughly half
/// a second of wall clock per emulated frame, so asking it the same question
/// twice in one comparison would double a five-minute run for nothing.
pub struct MisterReference {
    corpus: PathBuf,
    cache: Option<(String, u32, Capture)>,
}

impl MisterReference {
    /// `None` unless verilator is available.
    pub fn discover(corpus: impl Into<PathBuf>) -> Option<MisterReference> {
        have_verilator().then(|| MisterReference {
            corpus: corpus.into(),
            cache: None,
        })
    }

    /// Drop any cached run, forcing the next call to re-simulate.
    pub fn forget(&mut self) {
        self.cache = None;
    }
}

impl Reference for MisterReference {
    fn run(&mut self, trace: &Trace, frames: u32, want_pixels: bool) -> Result<Vec<Frame>, String> {
        let key = trace.serialise();
        let fresh = match &self.cache {
            Some((k, n, _)) if *k == key && *n >= frames => false,
            _ => true,
        };
        if fresh {
            let capture = capture_trace(&self.corpus, trace, frames)?;
            // Visible under --nocapture: flat 20480 deltas are what say the
            // stamp means what it claims.
            let deltas: Vec<u64> = capture.cycles.windows(2).take(4).map(|w| w[1] - w[0]).collect();
            eprintln!(
                "mister cycles:    first {:?}, deltas {:?}",
                &capture.cycles[..capture.cycles.len().min(4)],
                deltas
            );
            self.cache = Some((key, frames, capture));
        }
        let (_, _, capture) = self.cache.as_ref().expect("just populated");
        Ok(capture
            .frames
            .iter()
            .zip(&capture.cycles)
            .take(frames as usize)
            .map(|(rgb, cycles)| Frame::new(rgb.clone(), want_pixels).at(*cycles))
            .collect())
    }
}
