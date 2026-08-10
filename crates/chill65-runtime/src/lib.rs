//! 6502 machine state and interpreter for Atari coin-op hardware.
//!
//! Phase 2 of the project: a headless runtime that runs Crystal Castles from
//! ROM images the Phase 1 assembler produces. No third-party dependencies, and
//! no host-driven state — everything clocked runs off the machine's own cycle
//! counter, so a run is reproducible byte for byte.
//!
//! # The interpreter is permanent
//!
//! Plan §5 describes the "creep line": routines migrate from interpreter to
//! compiled Rust one at a time, both sharing a single machine state, so the
//! game is playable throughout and the blast radius of any bug is one routine.
//! That only works if the boundary is real, which is why [`cpu::Cpu`] is
//! entirely public and every peripheral hangs off the [`bus::Bus`] trait rather
//! than hiding inside the interpreter.
//!
//! # Layout
//!
//! - [`bus`] — the CPU's view of memory and I/O, plus a flat bus for tests.
//! - [`cpu`] — registers, flags, vectors, reset.
//! - [`addr`] — addressing-mode resolution, including the hardware quirks.
//! - [`exec`] — decode and execute.
//! - [`machine`] — the Crystal Castles bus: RAM, ROM banking, latches, watchdog.
//! - [`video`] — the bitmap coordinate window, colour RAM, scroll, framebuffer.
//! - [`frame`] — the clock derivation, frame timing and interrupt schedule.
//! - [`pokey`] — the two sound chips' registers and the RANDOM generator.
//! - [`input`] — switches and the trackball counters, plus the host API.
//!
//! POKEY and input arrive in subsequent tasks; see `LOOP.md`.

pub mod addr;
pub mod bus;
pub mod cpu;
pub mod exec;
pub mod frame;
pub mod input;
pub mod machine;
pub mod pokey;
pub mod video;

pub use addr::{Mode, Operand};
pub use bus::{Bus, FlatBus};
pub use cpu::{Cpu, CpuError};
pub use frame::{run_frame, run_frame_with, Compiled, FrameStats, NoCompiled};
pub use input::{Input, Switch};
pub use machine::Machine;
pub use pokey::Pokey;
pub use video::Video;
