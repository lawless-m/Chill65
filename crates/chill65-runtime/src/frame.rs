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

    for line in 0..LINES_PER_FRAME {
        // VBLANK is bit 5 of the switch port and the game polls it, so it must
        // track the scanline (SyncChain.v:43 — high for lines 0-23).
        machine.input.vblank = line < FIRST_VISIBLE_LINE;

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
        }
    }

    stats.cycles = machine.cycles - start;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;

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
}
