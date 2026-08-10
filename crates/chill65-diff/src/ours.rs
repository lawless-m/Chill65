//! Our own runtime as a [`Reference`] — the first implementation of the seam,
//! and the one every other oracle is compared against.

use chill65_runtime::frame::{run_frame_with, Compiled, NoCompiled};
use chill65_runtime::{Cpu, Machine};

use crate::reference::{rgb_from_indices, Frame, Reference};
use crate::trace::{FrameInput, Trace, ALL_SWITCHES};

/// One frame's picture together with the instruction that last wrote each
/// pixel — what routine attribution needs.
pub struct Snapshot {
    /// Canonical 256x232 RGB24.
    pub rgb: Vec<u8>,
    /// Per pixel, the PC of the last store that wrote it, indexed identically.
    pub writers: Vec<Option<u16>>,
}

/// Playing a trace through: the hash stream, and whether the machine stayed
/// healthy while producing it.
///
/// The watchdog matters more than it looks. If the game crashes, a real board
/// resets itself and carries on, so a crashed run still *looks* alive and still
/// produces a full stream of frames. An expired watchdog is the only signal
/// that happened.
pub struct Playback {
    pub hashes: Vec<u64>,
    pub cycles: u64,
    pub watchdog_expired: bool,
    /// Instructions the interpreter executed.
    pub interpreted: u64,
    /// Instructions compiled routines executed. Zero without a dispatch.
    pub compiled: u64,
}

/// The Crystal Castles machine model, driven from a trace.
pub struct OurRuntime {
    prog: Vec<u8>,
    data: Vec<u8>,
}

impl OurRuntime {
    /// `prog` must be 24576 bytes and `data` 16384; [`Machine::load_roms`]
    /// enforces it.
    pub fn new(prog: Vec<u8>, data: Vec<u8>) -> Self {
        OurRuntime { prog, data }
    }

    /// Run `frames` frames from cold, calling `per_frame` after each, and hand
    /// back the machine as it finished.
    fn drive(
        &self,
        trace: &Trace,
        frames: u32,
        log_writes: bool,
        per_frame: impl FnMut(&Machine),
    ) -> Result<Machine, String> {
        self.drive_with(trace, frames, log_writes, &mut NoCompiled, per_frame)
            .map(|(m, _)| m)
    }

    /// As [`OurRuntime::drive`], but offering each instruction boundary to a
    /// [`Compiled`] dispatch. With [`NoCompiled`] the two are identical, which
    /// is what makes a migrated routine checkable: run the same trace both ways
    /// and the hash streams must match.
    fn drive_with(
        &self,
        trace: &Trace,
        frames: u32,
        log_writes: bool,
        compiled: &mut impl Compiled,
        mut per_frame: impl FnMut(&Machine),
    ) -> Result<(Machine, u64), String> {
        let mut machine = Machine::new();
        machine.load_roms(&self.prog, &self.data)?;
        if log_writes {
            machine.enable_write_log();
        }
        let mut cpu = Cpu::new();
        cpu.reset(&mut machine);
        let mut interpreted = 0u64;

        for k in 0..frames {
            let input = trace
                .frames
                .get(k as usize)
                .copied()
                .unwrap_or_else(FrameInput::idle);

            // Switches are levels: every switch is set every frame, so a
            // release is as explicit as a press.
            for switch in ALL_SWITCHES {
                machine
                    .input
                    .set_switch(switch, input.switches.contains(switch));
            }
            machine.input.add_trackball_delta(input.dx, input.dy);

            // `run_frame` latches the accumulated movement itself, once, at the
            // frame boundary (frame.rs:115).
            let stats = run_frame_with(&mut cpu, &mut machine, compiled)
                .map_err(|e| format!("{e} at frame {k}, PC {:04X}", cpu.pc))?;
            interpreted += stats.interpreted;

            per_frame(&machine);
        }
        Ok((machine, interpreted))
    }

    /// Play a whole trace, reporting the hash stream and the machine's health.
    pub fn play(&mut self, trace: &Trace, frames: u32) -> Result<Playback, String> {
        self.play_with(trace, frames, &mut NoCompiled)
    }

    /// Play a whole trace with a compiled dispatch in place.
    pub fn play_with(
        &mut self,
        trace: &Trace,
        frames: u32,
        compiled: &mut impl Compiled,
    ) -> Result<Playback, String> {
        let mut hashes = Vec::with_capacity(frames as usize);
        let (machine, interpreted) = self.drive_with(trace, frames, false, compiled, |m| {
            hashes.push(Frame::new(rgb_from_indices(&m.framebuffer(), &m.video.cram), false).hash);
        })?;
        Ok(Playback {
            hashes,
            cycles: machine.cycles,
            watchdog_expired: machine.watchdog_expired,
            interpreted,
            compiled: compiled.compiled_instructions(),
        })
    }

    /// Run as far as `frame` inclusive and capture it, with attribution.
    ///
    /// The write log runs for the *whole* run, not just the last frame, which
    /// is the point: the bitmap is never cleared, so a pixel that differs at
    /// frame 400 may have been drawn at frame 12 and left alone since. The
    /// blame belongs to whoever last wrote it, however long ago that was.
    pub fn snapshot(&mut self, trace: &Trace, frame: u32) -> Result<Snapshot, String> {
        let machine = self.drive(trace, frame + 1, true, |_| {})?;
        Ok(Snapshot {
            rgb: rgb_from_indices(&machine.framebuffer(), &machine.video.cram),
            writers: machine.pixel_writers().expect("the log was enabled"),
        })
    }
}

impl Reference for OurRuntime {
    fn run(&mut self, trace: &Trace, frames: u32, want_pixels: bool) -> Result<Vec<Frame>, String> {
        let mut out = Vec::with_capacity(frames as usize);
        self.drive(trace, frames, false, |machine| {
            let rgb = rgb_from_indices(&machine.framebuffer(), &machine.video.cram);
            out.push(Frame::new(rgb, want_pixels));
        })?;
        Ok(out)
    }
}
