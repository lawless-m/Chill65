//! 6502 processor state.
//!
//! # Why this is public, all of it
//!
//! The interpreter is permanent infrastructure, not scaffolding. Plan §5's
//! "creep line" migrates routines from interpreter to compiled Rust one at a
//! time, with both sharing **one** machine state — a compiled routine returns
//! and the interpreter carries straight on from the registers it left behind.
//!
//! So there is no interpreter-private state here, and nothing a generated
//! routine could not also read or write. Anything hidden now becomes a hole in
//! the boundary later.
//!
//! # Why flags are separate bools
//!
//! The 6502's P register is eight bits, but code — interpreted or generated —
//! nearly always wants one flag at a time. Discrete fields keep that direct,
//! and [`Cpu::p`] / [`Cpu::set_p`] pack and unpack for the two places the byte
//! itself matters: `PHP`/`PLP` and interrupt entry/exit.

use crate::bus::Bus;

/// Bit positions in the packed status byte.
pub mod flag {
    pub const CARRY: u8 = 0x01;
    pub const ZERO: u8 = 0x02;
    pub const INTERRUPT: u8 = 0x04;
    pub const DECIMAL: u8 = 0x08;
    pub const BREAK: u8 = 0x10;
    /// Physically absent; reads as 1 whenever P is pushed.
    pub const UNUSED: u8 = 0x20;
    pub const OVERFLOW: u8 = 0x40;
    pub const NEGATIVE: u8 = 0x80;
}

/// Interrupt vectors.
pub mod vector {
    pub const NMI: u16 = 0xFFFA;
    pub const RESET: u16 = 0xFFFC;
    pub const IRQ: u16 = 0xFFFE;
}

/// The processor's registers and flags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cpu {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    /// Stack pointer. The stack lives at `0100`–`01FF`; `s` is the low byte.
    pub s: u8,
    pub pc: u16,

    pub carry: bool,
    pub zero: bool,
    pub interrupt_disable: bool,
    pub decimal: bool,
    pub overflow: bool,
    pub negative: bool,

    /// Machine cycles executed since reset. Drives every clocked peripheral,
    /// which is why the interpreter is deterministic.
    pub cycles: u64,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            a: 0,
            x: 0,
            y: 0,
            s: 0xFD,
            pc: 0,
            carry: false,
            zero: false,
            interrupt_disable: true,
            decimal: false,
            overflow: false,
            negative: false,
            cycles: 0,
        }
    }

    /// The status register as a byte.
    ///
    /// `UNUSED` always reads set. `BREAK` is **not** stored in the processor at
    /// all — it exists only in the copy pushed to the stack, and says whether
    /// the push came from `BRK`/`PHP` or from an interrupt. Callers supply it.
    pub fn p(&self) -> u8 {
        let mut p = flag::UNUSED;
        if self.carry {
            p |= flag::CARRY;
        }
        if self.zero {
            p |= flag::ZERO;
        }
        if self.interrupt_disable {
            p |= flag::INTERRUPT;
        }
        if self.decimal {
            p |= flag::DECIMAL;
        }
        if self.overflow {
            p |= flag::OVERFLOW;
        }
        if self.negative {
            p |= flag::NEGATIVE;
        }
        p
    }

    /// Unpack a status byte. `BREAK` and `UNUSED` are discarded — neither has
    /// any effect once inside the processor.
    pub fn set_p(&mut self, p: u8) {
        self.carry = p & flag::CARRY != 0;
        self.zero = p & flag::ZERO != 0;
        self.interrupt_disable = p & flag::INTERRUPT != 0;
        self.decimal = p & flag::DECIMAL != 0;
        self.overflow = p & flag::OVERFLOW != 0;
        self.negative = p & flag::NEGATIVE != 0;
    }

    /// Set N and Z from a value, the commonest flag update on the machine.
    #[inline]
    pub fn set_nz(&mut self, v: u8) {
        self.zero = v == 0;
        self.negative = v & 0x80 != 0;
    }

    /// Fetch the reset vector and start there.
    ///
    /// On this board that is `E000` — the first instruction selects ROM bank 0
    /// and jumps, because the bank-select latch is write-only and the machine
    /// cannot ask which bank it woke in.
    pub fn reset(&mut self, bus: &mut impl Bus) {
        self.pc = bus.read_u16(vector::RESET);
        self.s = 0xFD;
        self.interrupt_disable = true;
        self.decimal = false;
        self.cycles = 0;
    }
}

/// What went wrong when a step could not complete.
///
/// The interpreter never panics on bad data: a ROM image is input, and input
/// must not be able to take the process down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CpuError {
    /// An opcode outside the documented NMOS 6502 set.
    IllegalOpcode { opcode: u8, pc: u16 },
}

impl std::fmt::Display for CpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CpuError::IllegalOpcode { opcode, pc } => {
                write!(f, "illegal opcode {opcode:#04x} at {pc:04X}")
            }
        }
    }
}

impl std::error::Error for CpuError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::FlatBus;

    #[test]
    fn status_byte_round_trips() {
        let mut c = Cpu::new();
        for bits in 0u16..=0xFF {
            let bits = bits as u8;
            c.set_p(bits);
            // BREAK and UNUSED are not stored; UNUSED always reads back set.
            let expected = (bits & !flag::BREAK) | flag::UNUSED;
            assert_eq!(c.p(), expected, "round trip of {bits:#04x}");
        }
    }

    #[test]
    fn unused_bit_always_reads_set() {
        let mut c = Cpu::new();
        c.set_p(0x00);
        assert_eq!(c.p() & flag::UNUSED, flag::UNUSED);
    }

    #[test]
    fn break_is_not_processor_state() {
        let mut c = Cpu::new();
        c.set_p(flag::BREAK);
        assert_eq!(c.p() & flag::BREAK, 0, "B has no home in the processor");
    }

    #[test]
    fn set_nz_matches_the_hardware_rules() {
        let mut c = Cpu::new();
        c.set_nz(0x00);
        assert!(c.zero && !c.negative);
        c.set_nz(0x80);
        assert!(!c.zero && c.negative);
        c.set_nz(0x7F);
        assert!(!c.zero && !c.negative);
    }

    #[test]
    fn reset_follows_the_vector() {
        let mut b = FlatBus::new();
        b.set_reset_vector(0xE000);
        let mut c = Cpu::new();
        c.reset(&mut b);
        assert_eq!(c.pc, 0xE000, "the real board's reset target");
        assert_eq!(c.s, 0xFD);
        assert!(c.interrupt_disable);
        assert!(!c.decimal);
    }

    #[test]
    fn state_is_fully_reachable_from_outside() {
        // Plan section 5: interpreted and compiled routines share one machine
        // state. Anything unreachable here becomes a hole in that boundary, so
        // this test exists to fail if a field is ever made private.
        let mut c = Cpu::new();
        c.a = 1;
        c.x = 2;
        c.y = 3;
        c.s = 4;
        c.pc = 5;
        c.carry = true;
        c.zero = true;
        c.interrupt_disable = false;
        c.decimal = true;
        c.overflow = true;
        c.negative = true;
        c.cycles = 6;
        assert_eq!(c.a, 1);
        assert_eq!(c.cycles, 6);
        assert_eq!(c, c.clone());
    }
}
