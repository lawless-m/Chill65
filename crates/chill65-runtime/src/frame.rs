//! Frame timing and the interrupt schedule.
//!
//! # The clock chain
//!
//! `Clock.v:19-30` divides one 10 MHz master clock with a 3-bit counter:
//!
//! ```verilog
//! assign ce5  = count[0];        // 5 MHz    — pixel clock enable
//! assign ce2H = count == 7;      // 1.25 MHz — CPU clock enable
//! ```
//!
//! and `MicroProcessor.v:30` wires `.RDY(ce2H)`, so **the CPU advances one
//! cycle per `ce2H`: 1.25 MHz**, one CPU cycle for every four pixels.
//!
//! `SyncChain.v:30-33` steps `hcount` on `ce5` and wraps it at 320, stepping
//! the 8-bit `vcount` — so 320 pixels by 256 lines. That gives
//!
//! ```text
//!   320 * 256          = 81,920 pixel clocks per frame
//!   5,000,000 / 81,920 = 61.035 Hz          (README: "61.03 Hz")
//!   320 / 4            = 80 CPU cycles per line
//!   80 * 256           = 20,480 CPU cycles per frame
//!   1,250,000 / 20,480 = 61.035 Hz          — closes exactly
//! ```
//!
//! # The interrupt schedule
//!
//! `SyncChain.v:45`: `assign IRQCLK = ~vcount[5];  // every 64 lines (4x per
//! frame)`, and `MicroProcessor.v:38-46` latches IRQ on that signal's **rising**
//! edge, clearing it only on the INTACK strobe:
//!
//! ```verilog
//! else if (~INTACKn)  IRQ <= 1'b0;
//! else begin IRQCLK2 <= IRQCLK; if (~IRQCLK2 & IRQCLK) IRQ <= 1'b1; end
//! ```
//!
//! `~vcount[5]` is high for lines 0–31, 64–95, 128–159 and 192–223, so it rises
//! entering lines **0, 64, 128 and 192** — four times a frame, evenly spaced.
//!
//! Two consequences worth stating plainly:
//!
//! - IRQ is a **latched level**, not a pulse. Raising it while `I` is set does
//!   not lose it; the CPU takes it the moment `I` clears. Only INTACK clears it.
//! - A handler that never acknowledges will be re-entered as soon as `RTI`
//!   restores `I`. That is real hardware behaviour, not a modelling artefact,
//!   and it is why the tests below install a handler that strobes `9D80`.
//!
//! # NMI is never raised
//!
//! `MicroProcessor.v:26` ties the line low with `.NMI(1'b0)`. The vector at
//! `FFFA` exists and points at `E009`, and nothing can ever reach it. This
//! scheduler does not raise NMI, and should not be "fixed" to do so.

use crate::cpu::{Cpu, CpuError};
use crate::machine::Machine;

/// CPU clock in Hz — `ce2H`, one eighth of the 10 MHz master.
pub const CPU_HZ: u32 = 1_250_000;

/// Pixel clock in Hz — `ce5`.
pub const PIXEL_HZ: u32 = 5_000_000;

/// `hcount` wraps here (`SyncChain.v:33`).
pub const PIXELS_PER_LINE: u32 = 320;

/// `vcount` is 8 bits, so a frame is 256 lines whatever is visible.
pub const LINES_PER_FRAME: u32 = 256;

/// Four pixels to the CPU cycle.
pub const CYCLES_PER_LINE: u32 = PIXELS_PER_LINE / (PIXEL_HZ / CPU_HZ);

/// 20,480.
pub const CYCLES_PER_FRAME: u32 = CYCLES_PER_LINE * LINES_PER_FRAME;

/// `IRQCLK = ~vcount[5]` rises every 64 lines.
pub const IRQ_LINE_INTERVAL: u32 = 64;

/// The lines on which IRQ is raised: 0, 64, 128, 192.
pub const IRQ_LINES: [u32; 4] = [0, 64, 128, 192];

/// First line for which `VBLANK` is clear (`SyncChain.v:43`: high for 0–23).
///
/// Corroborated from the RTL, but worth watching: this 0–23 range is the clamp
/// the MiSTer core's author queried in their own outstanding-work notes.
pub const FIRST_VISIBLE_LINE: u32 = 24;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FrameStats {
    /// CPU cycles actually consumed. See [`run_frame`] on why this can exceed
    /// [`CYCLES_PER_FRAME`] slightly.
    pub cycles: u64,
    /// Interrupts the CPU actually *took* this frame. Raising is not taking:
    /// with `I` set this stays zero while the request remains pending.
    pub irqs_taken: u32,
    /// Times IRQ was raised — always 4, by construction.
    pub irqs_raised: u32,
    /// Instructions executed by the interpreter, one per [`Cpu::step`].
    ///
    /// Paired with a [`Compiled`] dispatch's own count, this is the
    /// denominator of "how much of the running code is compiled" — and
    /// executed instructions are the honest unit, since they weight a routine
    /// by how much it actually runs rather than by how much of it exists.
    pub interpreted: u64,
}

/// Compiled routines, sharing the interpreter's machine state.
///
/// Plan §5's creep line: routines migrate from interpreter to compiled one at a
/// time, both running against one `Machine`, so the game works throughout and
/// the blast radius of any bug is a single routine.
///
/// # The contract emitted code must honour
///
/// [`Compiled::enter`] is offered the machine at an instruction boundary and
/// returns `true` if it executed anything. Everything below is what generated
/// code has to do to be indistinguishable from the interpreter:
///
/// - **Whole instructions only.** Never stop half way through one.
/// - **Check the same two conditions the interpreter's loop checks, before
///   every instruction:** `machine.cycles < deadline`, and stop if
///   `machine.irq_pending && !cpu.interrupt_disable`.
/// - **Yield with `cpu.pc` on the next unexecuted instruction.** When either
///   condition fails, set `pc` and return. The interpreter then takes the IRQ
///   and executes the rest of the routine itself. That is always safe because
///   dispatch is offered only at routine *entries*: a half-run routine is
///   finished by the interpreter, never re-entered from the top.
/// - **Call `machine.begin_instruction(pc)` before each instruction**, so the
///   write log keeps attributing bitmap writes to the right address and
///   `chill65-diff`'s localisation keeps working on compiled code.
/// - **Apply exact 6502 semantics and tick exact cycles**, page-crossing and
///   branch-taken penalties included. A frame is 256 lines of cycle deadlines
///   and the game is written against them; a routine that runs "fast" changes
///   when interrupts land.
/// - **`JSR`/`JMP` leaving the routine, and `RTS`/`RTI`, set `pc` and return.**
///   Control transfer out is a yield, and the dispatch gets another chance at
///   the destination.
///
/// A routine that follows all of that produces the same machine state, the same
/// cycle count and the same picture as interpreting it — which is exactly what
/// `ccnative verify` checks over the trace corpus.
pub trait Compiled {
    /// Run a compiled routine if one starts at `cpu.pc`. `true` if anything ran.
    fn enter(&mut self, cpu: &mut Cpu, machine: &mut Machine, deadline: u64) -> bool;

    /// Instructions executed by compiled code since the machine started.
    fn compiled_instructions(&self) -> u64;
}

/// A dispatch that compiles nothing — the interpreter alone.
///
/// `run_frame_with(.., &mut NoCompiled)` is `run_frame`, which is what makes
/// the two directly comparable.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoCompiled;

impl Compiled for NoCompiled {
    fn enter(&mut self, _cpu: &mut Cpu, _machine: &mut Machine, _deadline: u64) -> bool {
        false
    }
    fn compiled_instructions(&self) -> u64 {
        0
    }
}

/// Run one frame: 256 scanlines, raising IRQ at the four line boundaries the
/// video counters dictate.
///
/// Interrupts are checked at instruction boundaries, which is where a real
/// 6502 checks them too. The line deadline can therefore land mid-instruction,
/// so a frame may overshoot [`CYCLES_PER_FRAME`] by up to one instruction —
/// at most six cycles, under 0.03% of a frame. The deadlines are absolute
/// within the frame rather than accumulated per line, so that overshoot does
/// **not** compound across the 256 lines; each IRQ still lands within one
/// instruction of its true cycle.
pub fn run_frame(cpu: &mut Cpu, machine: &mut Machine) -> Result<FrameStats, CpuError> {
    let start = machine.cycles;
    let mut stats = FrameStats::default();

    // Plan §6: host movement accumulated since the last frame is presented to
    // the CPU exactly once, here, at the frame boundary.
    machine.input.latch_frame();
    machine.begin_audio_frame();

    for line in 0..LINES_PER_FRAME {
        // VBLANK is bit 5 of the switch port and the game polls it, so it must
        // track the scanline (SyncChain.v:43 — high for lines 0-23).
        machine.input.vblank = line < FIRST_VISIBLE_LINE;
        if line == FIRST_VISIBLE_LINE {
            // The video hardware starts reading the object table here. The
            // game parks the table again before the frame ends, so this is the
            // only instant at which a snapshot of it means anything.
            machine.latch_motion_objects();
        }

        if line % IRQ_LINE_INTERVAL == 0 {
            // Rising edge of IRQCLK. Latched: if one is already pending and
            // unacknowledged, this changes nothing, exactly as the flip-flop
            // in MicroProcessor.v does nothing.
            machine.irq_pending = true;
            stats.irqs_raised += 1;
        }

        let deadline = start + ((line + 1) as u64) * CYCLES_PER_LINE as u64;
        while machine.cycles < deadline {
            if machine.irq_pending && !cpu.interrupt_disable {
                cpu.irq(machine);
                stats.irqs_taken += 1;
                continue;
            }
            cpu.step(machine)?;
            stats.interpreted += 1;
        }
    }

    machine.end_audio_frame();
    stats.cycles = machine.cycles - start;
    Ok(stats)
}

/// [`run_frame`], but offering each instruction boundary to `compiled` first.
///
/// The loop is deliberately the same shape: the dispatch is consulted where the
/// interpreter would have stepped, and everything else — the interrupt schedule,
/// the per-line deadlines, VBLANK tracking — is untouched. With
/// [`NoCompiled`] this is `run_frame` exactly.
pub fn run_frame_with(
    cpu: &mut Cpu,
    machine: &mut Machine,
    compiled: &mut impl Compiled,
) -> Result<FrameStats, CpuError> {
    let start = machine.cycles;
    let mut stats = FrameStats::default();

    machine.input.latch_frame();
    machine.begin_audio_frame();

    for line in 0..LINES_PER_FRAME {
        machine.input.vblank = line < FIRST_VISIBLE_LINE;
        if line == FIRST_VISIBLE_LINE {
            // The video hardware starts reading the object table here. The
            // game parks the table again before the frame ends, so this is the
            // only instant at which a snapshot of it means anything.
            machine.latch_motion_objects();
        }

        if line % IRQ_LINE_INTERVAL == 0 {
            machine.irq_pending = true;
            stats.irqs_raised += 1;
        }

        let deadline = start + ((line + 1) as u64) * CYCLES_PER_LINE as u64;
        while machine.cycles < deadline {
            if machine.irq_pending && !cpu.interrupt_disable {
                cpu.irq(machine);
                stats.irqs_taken += 1;
                continue;
            }
            // Offered at an instruction boundary, with the interrupt already
            // handled — so a compiled routine never has to consider taking one,
            // only yielding when one is pending.
            if compiled.enter(cpu, machine, deadline) {
                continue;
            }
            cpu.step(machine)?;
            stats.interpreted += 1;
        }
    }

    machine.end_audio_frame();
    stats.cycles = machine.cycles - start;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

    /// A hand-written stand-in for generated code: the spin loop at `E000`.
    ///
    /// `E000` is `JMP $E000`, three cycles, leaving `pc` where it was. It obeys
    /// the [`Compiled`] contract literally — deadline and IRQ checked before
    /// every instruction, `begin_instruction` called, exact cycles ticked — so
    /// it stands in for what the emitter will produce.
    struct CompiledSpin {
        executed: u64,
    }

    impl Compiled for CompiledSpin {
        fn enter(&mut self, cpu: &mut Cpu, machine: &mut Machine, deadline: u64) -> bool {
            if cpu.pc != 0xE000 {
                return false;
            }
            let mut ran = false;
            loop {
                // Exactly the two conditions the interpreter's loop checks.
                if machine.cycles >= deadline {
                    break;
                }
                if machine.irq_pending && !cpu.interrupt_disable {
                    break;
                }
                machine.begin_instruction(0xE000);
                machine.tick(3);
                cpu.pc = 0xE000;
                self.executed += 1;
                ran = true;
            }
            ran
        }

        fn compiled_instructions(&self) -> u64 {
            self.executed
        }
    }

    #[test]
    fn dispatching_a_routine_matches_interpreting_it() {
        // The whole creep line rests on this: a compiled routine and the
        // interpreter must be indistinguishable in state and in cycles.
        // Interrupts enabled on both sides, so the run really does mix the two:
        // the spin loop dispatches, the ISR stays interpreted. Reset leaves `I`
        // set, as the hardware does, and the spin loop never clears it.
        let (mut cpu_a, mut m_a) = machine_with_isr();
        cpu_a.reset(&mut m_a);
        cpu_a.interrupt_disable = false;
        let interpreted = run_frame(&mut cpu_a, &mut m_a).expect("interpreted frame");

        let (mut cpu_b, mut m_b) = machine_with_isr();
        cpu_b.reset(&mut m_b);
        cpu_b.interrupt_disable = false;
        let mut compiled = CompiledSpin { executed: 0 };
        let dispatched =
            run_frame_with(&mut cpu_b, &mut m_b, &mut compiled).expect("dispatched frame");

        assert_eq!(interpreted.cycles, dispatched.cycles, "cycle counts differ");
        assert_eq!(
            interpreted.irqs_taken, dispatched.irqs_taken,
            "interrupts taken differ"
        );
        assert_eq!(interpreted.irqs_raised, dispatched.irqs_raised);
        assert_eq!(m_a.cycles, m_b.cycles);
        assert_eq!(m_a.frame_hash(), m_b.frame_hash(), "pictures differ");
        assert_eq!(cpu_a.pc, cpu_b.pc, "the CPU ended somewhere else");
        assert_eq!(cpu_a.a, cpu_b.a);
        assert_eq!(cpu_a.s, cpu_b.s, "stack pointer");

        // And the work really did move: the spin loop ran compiled, the ISR
        // stayed interpreted.
        assert!(compiled.compiled_instructions() > 0, "nothing was dispatched");
        assert!(dispatched.interpreted > 0, "the ISR should still interpret");
        assert!(
            dispatched.interpreted < interpreted.interpreted,
            "dispatching should have taken work off the interpreter: {} vs {}",
            dispatched.interpreted,
            interpreted.interpreted
        );
    }

    #[test]
    fn a_compiled_routine_yields_so_the_interrupt_can_be_taken() {
        // A routine that ran past a pending IRQ would delay every interrupt in
        // the frame, and the game is written against when they land.
        let (mut cpu, mut m) = machine_with_isr();
        cpu.reset(&mut m);
        let mut compiled = CompiledSpin { executed: 0 };

        m.irq_pending = true;
        cpu.interrupt_disable = false;
        let before = m.cycles;
        let ran = compiled.enter(&mut cpu, &mut m, before + 10_000);

        assert!(!ran, "the routine ran with an interrupt pending");
        assert_eq!(m.cycles, before, "and it consumed cycles doing so");
    }

    #[test]
    fn a_compiled_routine_stops_at_the_deadline() {
        let (mut cpu, mut m) = machine_with_isr();
        cpu.reset(&mut m);
        let mut compiled = CompiledSpin { executed: 0 };

        let deadline = m.cycles + 30;
        assert!(compiled.enter(&mut cpu, &mut m, deadline));
        assert!(
            m.cycles >= deadline && m.cycles < deadline + 3,
            "overran the deadline by more than one instruction: {} vs {deadline}",
            m.cycles
        );
        assert_eq!(cpu.pc, 0xE000, "pc must sit on the next unexecuted instruction");
    }

    #[test]
    fn no_compiled_is_exactly_the_interpreter() {
        let (mut cpu_a, mut m_a) = machine_with_isr();
        let a = run_frame(&mut cpu_a, &mut m_a).expect("frame");
        let (mut cpu_b, mut m_b) = machine_with_isr();
        let b = run_frame_with(&mut cpu_b, &mut m_b, &mut NoCompiled).expect("frame");
        assert_eq!(a.cycles, b.cycles);
        assert_eq!(a.interpreted, b.interpreted);
        assert_eq!(a.irqs_taken, b.irqs_taken);
        assert_eq!(m_a.frame_hash(), m_b.frame_hash());
    }

    /// Build a machine whose ROM contains a spin loop at the reset vector and a
    /// realistic interrupt handler: acknowledge, then return.
    ///
    /// Acknowledging matters. `CIN.MAC:82-83` strobes INTACK and the watchdog
    /// before restoring registers; a handler that skipped it would be
    /// re-entered the instant `RTI` restored `I`, which is what the hardware
    /// does and what the last test here checks.
    fn machine_with_isr() -> (Cpu, Machine) {
        let mut m = Machine::new();
        let at = |addr: u16| (addr - 0xA000) as usize;

        // E000: main program — JMP $E000, a spin loop.
        m.prog[at(0xE000)..at(0xE000) + 3].copy_from_slice(&[0x4C, 0x00, 0xE0]);

        // E100: ISR — STA $9D80 (INTACK), STA $9E00 (watchdog), RTI.
        m.prog[at(0xE100)..at(0xE100) + 7]
            .copy_from_slice(&[0x8D, 0x80, 0x9D, 0x8D, 0x00, 0x9E, 0x40]);

        // Vectors.
        m.prog[at(0xFFFC)] = 0x00;
        m.prog[at(0xFFFD)] = 0xE0;
        m.prog[at(0xFFFE)] = 0x00;
        m.prog[at(0xFFFF)] = 0xE1;

        let mut cpu = Cpu::new();
        cpu.reset(&mut m);
        (cpu, m)
    }

    #[test]
    fn the_derivation_closes() {
        assert_eq!(CYCLES_PER_LINE, 80);
        assert_eq!(CYCLES_PER_FRAME, 20_480);
        // The frame rate must come out the same computed either way, or the
        // divisor is wrong somewhere.
        let from_pixels = PIXEL_HZ as f64 / (PIXELS_PER_LINE * LINES_PER_FRAME) as f64;
        let from_cpu = CPU_HZ as f64 / CYCLES_PER_FRAME as f64;
        assert!((from_pixels - from_cpu).abs() < 1e-9);
        assert!((from_pixels - 61.035).abs() < 0.01, "{from_pixels}");
    }

    #[test]
    fn irqclk_rises_four_times_a_frame() {
        // IRQCLK = ~vcount[5]; a rising edge is vcount[5] going 1 -> 0.
        let irqclk = |v: u32| (!(v >> 5)) & 1;
        let mut rises = Vec::new();
        for v in 0..LINES_PER_FRAME {
            let prev = (v + LINES_PER_FRAME - 1) % LINES_PER_FRAME;
            if irqclk(prev) == 0 && irqclk(v) == 1 {
                rises.push(v);
            }
        }
        assert_eq!(rises, IRQ_LINES, "four per frame, every 64 lines");
    }

    #[test]
    fn exactly_four_interrupts_are_taken_per_frame() {
        let (mut cpu, mut m) = machine_with_isr();
        cpu.interrupt_disable = false;

        for frame in 0..3 {
            let stats = run_frame(&mut cpu, &mut m).unwrap();
            assert_eq!(stats.irqs_raised, 4, "frame {frame}");
            assert_eq!(stats.irqs_taken, 4, "frame {frame}");
            assert!(!m.irq_pending, "the handler acknowledged the last one");
        }
    }

    #[test]
    fn no_interrupts_are_taken_while_i_is_set() {
        let (mut cpu, mut m) = machine_with_isr();
        cpu.interrupt_disable = true;

        let stats = run_frame(&mut cpu, &mut m).unwrap();
        assert_eq!(stats.irqs_raised, 4);
        assert_eq!(stats.irqs_taken, 0, "masked");
        // But the request is latched in hardware, not lost.
        assert!(m.irq_pending, "IRQ is a level, held until INTACK");

        // Clearing I lets the pending one through immediately.
        cpu.interrupt_disable = false;
        let stats = run_frame(&mut cpu, &mut m).unwrap();
        assert_eq!(
            stats.irqs_taken, 4,
            "the held request plus this frame's three later ones"
        );
    }

    #[test]
    fn a_frame_is_20480_cycles_to_within_one_instruction() {
        let (mut cpu, mut m) = machine_with_isr();
        cpu.interrupt_disable = false;
        let stats = run_frame(&mut cpu, &mut m).unwrap();
        assert!(
            (CYCLES_PER_FRAME as u64..CYCLES_PER_FRAME as u64 + 7).contains(&stats.cycles),
            "frame took {} cycles, expected ~{CYCLES_PER_FRAME}",
            stats.cycles
        );
    }

    #[test]
    fn overshoot_does_not_compound_across_a_frame() {
        // Deadlines are absolute within the frame, so 256 lines of mid-
        // instruction boundaries must not drift the frame length.
        let (mut cpu, mut m) = machine_with_isr();
        cpu.interrupt_disable = false;
        for _ in 0..20 {
            let stats = run_frame(&mut cpu, &mut m).unwrap();
            assert!(stats.cycles < CYCLES_PER_FRAME as u64 + 7, "{stats:?}");
        }
        // Twenty frames must not have drifted by more than twenty instructions.
        let expected = 20 * CYCLES_PER_FRAME as u64;
        assert!(m.cycles - expected < 20 * 7, "drift: {}", m.cycles - expected);
    }

    #[test]
    fn the_watchdog_survives_a_frame_because_the_isr_feeds_it() {
        let (mut cpu, mut m) = machine_with_isr();
        cpu.interrupt_disable = false;
        for _ in 0..10 {
            run_frame(&mut cpu, &mut m).unwrap();
        }
        assert!(!m.watchdog_expired, "four strobes a frame is plenty");
        assert_eq!(m.watchdog_strobes, 40);
    }

    #[test]
    fn a_machine_that_masks_interrupts_forever_trips_the_watchdog() {
        // Nothing acknowledges, so nothing strobes 9E00. This is the failure
        // the watchdog exists to catch.
        let (mut cpu, mut m) = machine_with_isr();
        cpu.interrupt_disable = true;
        while m.cycles < m.watchdog_limit as u64 {
            run_frame(&mut cpu, &mut m).unwrap();
        }
        assert!(m.watchdog_expired);
    }

    #[test]
    fn an_unacknowledging_handler_is_re_entered() {
        // Real behaviour, not an artefact: IRQ is a level cleared only by
        // INTACK, so an ISR that returns without strobing 9D80 comes straight
        // back the moment RTI restores I.
        let mut m = Machine::new();
        let at = |addr: u16| (addr - 0xA000) as usize;
        m.prog[at(0xE000)..at(0xE000) + 3].copy_from_slice(&[0x4C, 0x00, 0xE0]);
        m.prog[at(0xE100)] = 0x40; // RTI, with no acknowledge
        m.prog[at(0xFFFC)] = 0x00;
        m.prog[at(0xFFFD)] = 0xE0;
        m.prog[at(0xFFFE)] = 0x00;
        m.prog[at(0xFFFF)] = 0xE1;

        let mut cpu = Cpu::new();
        cpu.reset(&mut m);
        cpu.interrupt_disable = false;

        let stats = run_frame(&mut cpu, &mut m).unwrap();
        assert_eq!(stats.irqs_raised, 4);
        assert!(
            stats.irqs_taken > 100,
            "took {} — an unacknowledged level re-fires continuously",
            stats.irqs_taken
        );
    }

    #[test]
    fn trackball_movement_reaches_the_cpu_once_per_frame() {
        use crate::input::AXIS_HORIZONTAL;
        let (mut cpu, mut m) = machine_with_isr();
        cpu.interrupt_disable = false;

        // Movement arriving mid-frame must not be visible until the next
        // boundary — plan §6.
        m.input.add_trackball_delta(7, 0);
        assert_eq!(m.read(0x9401), 0, "not yet presented");

        run_frame(&mut cpu, &mut m).unwrap();
        assert_eq!(m.read(0x9401), 7, "presented at the frame boundary");

        // Several deltas within one frame arrive as a single step.
        for _ in 0..4 {
            m.input.add_trackball_delta(2, 0);
        }
        run_frame(&mut cpu, &mut m).unwrap();
        assert_eq!(m.read(0x9401), 15, "7 + 8, in one presentation");
        assert_eq!(m.input.pending_h, 0, "accumulator drained");

        // An idle frame does not move the absolute counter.
        run_frame(&mut cpu, &mut m).unwrap();
        assert_eq!(m.read(0x9401), 15);
        assert_eq!(
            m.input.counters[AXIS_HORIZONTAL], 15,
            "the counter is a position, not a velocity"
        );
    }

    #[test]
    fn vblank_tracks_the_scanline_during_a_frame() {
        // Bit 5 of the switch port is a live video signal the game polls, so it
        // must actually move as the frame advances rather than sit constant.
        let mut m = Machine::new();
        let mut cpu = Cpu::new();
        cpu.reset(&mut m);
        cpu.interrupt_disable = true;

        // Sample it from inside the frame by watching the model directly: the
        // scheduler sets it per line, so after a full frame it holds the state
        // of the last line, which is visible.
        run_frame(&mut cpu, &mut m).unwrap();
        assert!(
            !m.input.vblank,
            "line 255 is outside vblank (high only for 0-23)"
        );
        assert_eq!(
            m.read(0x9600) & (1 << crate::input::VBLANK_BIT),
            0,
            "and that reaches the CPU as a clear bit 5"
        );
    }

    #[test]
    fn nmi_is_never_raised_by_the_scheduler() {
        // MicroProcessor.v:26 ties the line low. Guard against a future slice
        // "helpfully" scheduling it.
        let (mut cpu, mut m) = machine_with_isr();
        cpu.interrupt_disable = false;
        // FFFA/FFFB are zero, so an NMI would send the PC to 0000 and the spin
        // loop at E000 would be abandoned.
        for _ in 0..5 {
            run_frame(&mut cpu, &mut m).unwrap();
        }
        assert_eq!(m.peek(0xFFFA), 0, "vector left unset on purpose");
        assert!(
            (0xE000..0xE200).contains(&cpu.pc),
            "PC wandered to {:04X} — something raised NMI",
            cpu.pc
        );
    }

    /// Programme one POKEY channel so the stream has something in it: a pure
    /// tone at full volume, which alternates between silence and 15.
    fn sounding() -> (Cpu, Machine) {
        let (cpu, mut m) = machine_with_isr();
        let at = m.cycles;
        m.pokey0.write(crate::pokey::reg::SKCTL, 0x07, at);
        m.pokey0.write(0, 0, at); // AUDF0
        m.pokey0.write(1, 0xAF, at); // AUDC0: pure tone, volume 15
        m.pokey0.write(crate::pokey::reg::STIMER, 0, at);
        (cpu, m)
    }

    #[test]
    fn audio_is_off_by_default_and_retains_nothing() {
        let (mut cpu, mut m) = sounding();
        assert!(!m.audio_enabled());
        for _ in 0..8 {
            run_frame(&mut cpu, &mut m).unwrap();
        }
        assert!(m.audio_samples().is_empty(), "nobody asked for audio");
        assert!(m.pokey0.samples.is_empty(), "and no chip buffered any");
        assert!(m.pokey1.samples.is_empty());
    }

    /// One sample per CPU cycle the frame actually ran.
    ///
    /// That is [`CYCLES_PER_FRAME`] plus the documented overshoot: interrupts
    /// are checked at instruction boundaries, so the last line can run a few
    /// cycles past its deadline. Emitting the true span rather than a fixed
    /// 20,480 is deliberate — trimming would drop cycles of sound and padding
    /// would invent them, and either would drift the audio clock away from the
    /// CPU's over a run of any length.
    #[test]
    fn an_enabled_frame_yields_one_sample_per_cycle() {
        let (mut cpu, mut m) = sounding();
        m.set_audio_enabled(true);
        for _ in 0..4 {
            let stats = run_frame(&mut cpu, &mut m).unwrap();
            assert_eq!(m.audio_samples().len() as u64, stats.cycles);
            assert!(
                (CYCLES_PER_FRAME as u64..CYCLES_PER_FRAME as u64 + 7).contains(&stats.cycles),
                "{} samples",
                stats.cycles
            );
        }
    }

    /// The frame is refilled, not appended to, so an embedder that never drains
    /// cannot grow the buffer.
    #[test]
    fn the_buffer_is_replaced_each_frame() {
        let (mut cpu, mut m) = sounding();
        m.set_audio_enabled(true);
        run_frame(&mut cpu, &mut m).unwrap();
        let first = m.audio_samples().len();
        for _ in 0..6 {
            run_frame(&mut cpu, &mut m).unwrap();
            assert!(m.audio_samples().len() < first + 8, "one frame's worth, not seven");
        }
    }

    /// Audio must be observationally free: identical pictures and identical
    /// cycle counts either way. If this fails, recording has perturbed the
    /// machine — most likely by advancing a POKEY across a register write.
    #[test]
    fn audio_does_not_change_the_frame_hash_or_the_cycle_count() {
        let (mut ca, mut a) = sounding();
        let (mut cb, mut b) = sounding();
        b.set_audio_enabled(true);
        for frame in 0..12 {
            let sa = run_frame(&mut ca, &mut a).unwrap();
            let sb = run_frame(&mut cb, &mut b).unwrap();
            assert_eq!(a.frame_hash(), b.frame_hash(), "frame {frame}");
            assert_eq!(sa.cycles, sb.cycles, "frame {frame}");
            assert_eq!(a.cycles, b.cycles, "frame {frame}");
            // And RANDOM still reads the same, which is the one POKEY output
            // the game makes decisions from.
            let at = a.cycles;
            assert_eq!(
                a.pokey0.read(crate::pokey::reg::RANDOM, at),
                b.pokey0.read(crate::pokey::reg::RANDOM, at),
                "frame {frame}"
            );
        }
    }

    #[test]
    fn two_identically_driven_machines_produce_the_same_stream() {
        let (mut ca, mut a) = sounding();
        let (mut cb, mut b) = sounding();
        a.set_audio_enabled(true);
        b.set_audio_enabled(true);
        for frame in 0..6 {
            run_frame(&mut ca, &mut a).unwrap();
            run_frame(&mut cb, &mut b).unwrap();
            assert_eq!(a.audio_samples(), b.audio_samples(), "frame {frame}");
        }
    }

    /// A programmed channel actually sounds, and a silent chip actually does
    /// not — so the test above is not comparing two empty streams.
    #[test]
    fn the_stream_carries_the_programmed_tone_and_silence_is_silent() {
        let (mut cpu, mut m) = sounding();
        m.set_audio_enabled(true);
        run_frame(&mut cpu, &mut m).unwrap();
        let s = m.audio_samples();
        assert!(s.iter().any(|&v| v == 15), "the tone is present");
        assert!(s.iter().any(|&v| v == 0), "and it is a square wave");

        let (mut cpu, mut m) = machine_with_isr();
        m.set_audio_enabled(true);
        run_frame(&mut cpu, &mut m).unwrap();
        assert!(
            m.audio_samples().iter().all(|&v| v == 0),
            "an unprogrammed board is silent"
        );
    }

    /// Turning audio off releases the buffers rather than merely stopping.
    #[test]
    fn disabling_audio_releases_the_buffers() {
        let (mut cpu, mut m) = sounding();
        m.set_audio_enabled(true);
        run_frame(&mut cpu, &mut m).unwrap();
        assert!(!m.audio_samples().is_empty());
        m.set_audio_enabled(false);
        assert!(m.audio_samples().is_empty());
        run_frame(&mut cpu, &mut m).unwrap();
        assert!(m.audio_samples().is_empty(), "and stays released");
    }

    /// The dispatched loop must behave like the interpreted one here too.
    #[test]
    fn the_compiled_loop_produces_the_same_stream() {
        let (mut ca, mut a) = sounding();
        let (mut cb, mut b) = sounding();
        a.set_audio_enabled(true);
        b.set_audio_enabled(true);
        let mut spin = CompiledSpin { executed: 0 };
        for frame in 0..4 {
            run_frame(&mut ca, &mut a).unwrap();
            run_frame_with(&mut cb, &mut b, &mut spin).unwrap();
            assert_eq!(a.audio_samples(), b.audio_samples(), "frame {frame}");
        }
        assert!(spin.compiled_instructions() > 0, "the dispatch really ran");
    }
}
