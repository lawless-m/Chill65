//! Our own runtime as a [`Reference`] — the first implementation of the seam,
//! and the one every other oracle is compared against.

use chill65_runtime::frame::run_frame;
use chill65_runtime::{Cpu, Machine};

use crate::reference::{rgb_from_indices, Frame, Reference};
use crate::trace::{FrameInput, Trace, ALL_SWITCHES};

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
}

impl Reference for OurRuntime {
    fn run(&mut self, trace: &Trace, frames: u32, want_pixels: bool) -> Result<Vec<Frame>, String> {
        let mut machine = Machine::new();
        machine.load_roms(&self.prog, &self.data)?;
        let mut cpu = Cpu::new();
        cpu.reset(&mut machine);

        let mut out = Vec::with_capacity(frames as usize);
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
            run_frame(&mut cpu, &mut machine)
                .map_err(|e| format!("{e} at frame {k}, PC {:04X}", cpu.pc))?;

            let rgb = rgb_from_indices(&machine.framebuffer(), &machine.video.cram);
            out.push(Frame::new(rgb, want_pixels));
        }
        Ok(out)
    }
}
