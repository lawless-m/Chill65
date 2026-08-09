//! Addressing-mode resolution.
//!
//! Three behaviours here are hardware quirks rather than design, and every one
//! of them is load-bearing for real 1983 code:
//!
//! - **Zero-page indexing wraps inside page zero.** `LDA $F0,X` with `X=$20`
//!   reads `$10`, not `$0110`. The game's zero page is crowded (`CG.MAC` packs
//!   variables from `$04` to `$EF` with a `.ERROR` guard at `$F0`) so indexed
//!   access near the top wraps in practice, not just in theory.
//! - **`JMP ($xxFF)` fetches its high byte from `$xx00`.** The 6502 does not
//!   carry into the high byte of the pointer.
//! - **Indirect vectors in page zero wrap too**: `(zp,X)` and `(zp),Y` read
//!   their high byte from `(ptr+1) & 0xFF`.
//!
//! Page crossings are reported because they cost a cycle on reads, and cycle
//! counts drive the interrupt schedule.

use crate::bus::Bus;
use crate::cpu::Cpu;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// No operand.
    Implied,
    /// Operates on the accumulator.
    Accumulator,
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    /// `JMP ($nnnn)` only.
    Indirect,
    /// `($nn,X)` — indexed indirect.
    IndirectX,
    /// `($nn),Y` — indirect indexed.
    IndirectY,
    /// Branch displacement.
    Relative,
}

/// A resolved operand address, and whether reaching it crossed a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Operand {
    pub addr: u16,
    /// Only ever set for the indexed modes, where it costs a read cycle.
    pub page_crossed: bool,
}

impl Cpu {
    /// Read the byte at PC and advance.
    #[inline]
    pub fn fetch(&mut self, bus: &mut impl Bus) -> u8 {
        let b = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        b
    }

    /// Read the little-endian word at PC and advance twice.
    #[inline]
    pub fn fetch_u16(&mut self, bus: &mut impl Bus) -> u16 {
        let lo = self.fetch(bus) as u16;
        let hi = self.fetch(bus) as u16;
        lo | (hi << 8)
    }

    /// Read a 16-bit pointer from page zero, wrapping within it.
    fn read_zp_ptr(&mut self, bus: &mut impl Bus, zp: u8) -> u16 {
        let lo = bus.read(zp as u16) as u16;
        let hi = bus.read(zp.wrapping_add(1) as u16) as u16;
        lo | (hi << 8)
    }

    /// Resolve `mode`, consuming its operand bytes from the instruction stream.
    ///
    /// Returns `None` for modes that have no address of their own — implied and
    /// accumulator.
    pub fn resolve(&mut self, bus: &mut impl Bus, mode: Mode) -> Option<Operand> {
        let plain = |addr: u16| {
            Some(Operand {
                addr,
                page_crossed: false,
            })
        };

        match mode {
            Mode::Implied | Mode::Accumulator => None,

            // The operand *is* the next byte; its address is where it sits.
            Mode::Immediate => {
                let addr = self.pc;
                self.pc = self.pc.wrapping_add(1);
                plain(addr)
            }

            Mode::ZeroPage => {
                let zp = self.fetch(bus);
                plain(zp as u16)
            }

            // Wraps inside page zero — never carries into page one.
            Mode::ZeroPageX => {
                let zp = self.fetch(bus);
                plain(zp.wrapping_add(self.x) as u16)
            }
            Mode::ZeroPageY => {
                let zp = self.fetch(bus);
                plain(zp.wrapping_add(self.y) as u16)
            }

            Mode::Absolute => {
                let addr = self.fetch_u16(bus);
                plain(addr)
            }

            Mode::AbsoluteX => {
                let base = self.fetch_u16(bus);
                let addr = base.wrapping_add(self.x as u16);
                Some(Operand {
                    addr,
                    page_crossed: (base & 0xFF00) != (addr & 0xFF00),
                })
            }
            Mode::AbsoluteY => {
                let base = self.fetch_u16(bus);
                let addr = base.wrapping_add(self.y as u16);
                Some(Operand {
                    addr,
                    page_crossed: (base & 0xFF00) != (addr & 0xFF00),
                })
            }

            // The famous bug: no carry into the pointer's high byte.
            Mode::Indirect => {
                let ptr = self.fetch_u16(bus);
                let lo = bus.read(ptr) as u16;
                let hi_addr = (ptr & 0xFF00) | ((ptr + 1) & 0x00FF);
                let hi = bus.read(hi_addr) as u16;
                plain(lo | (hi << 8))
            }

            Mode::IndirectX => {
                let zp = self.fetch(bus);
                let ptr = zp.wrapping_add(self.x);
                let addr = self.read_zp_ptr(bus, ptr);
                plain(addr)
            }

            Mode::IndirectY => {
                let zp = self.fetch(bus);
                let base = self.read_zp_ptr(bus, zp);
                let addr = base.wrapping_add(self.y as u16);
                Some(Operand {
                    addr,
                    page_crossed: (base & 0xFF00) != (addr & 0xFF00),
                })
            }

            Mode::Relative => {
                let disp = self.fetch(bus) as i8 as i16;
                let target = (self.pc as i16).wrapping_add(disp) as u16;
                Some(Operand {
                    addr: target,
                    page_crossed: (self.pc & 0xFF00) != (target & 0xFF00),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::FlatBus;

    fn setup(prog: &[u8]) -> (Cpu, FlatBus) {
        let mut bus = FlatBus::new();
        bus.load(0x0200, prog);
        let mut cpu = Cpu::new();
        cpu.pc = 0x0200;
        (cpu, bus)
    }

    #[test]
    fn zero_page_indexing_wraps_within_page_zero() {
        // LDA $F0,X with X=$20 must read $10, not $0110.
        let (mut c, mut b) = setup(&[0xF0]);
        c.x = 0x20;
        let op = c.resolve(&mut b, Mode::ZeroPageX).unwrap();
        assert_eq!(op.addr, 0x0010, "must wrap inside page zero");

        let (mut c, mut b) = setup(&[0xF0]);
        c.y = 0x20;
        assert_eq!(c.resolve(&mut b, Mode::ZeroPageY).unwrap().addr, 0x0010);
    }

    #[test]
    fn jmp_indirect_does_not_carry_into_the_high_byte() {
        // JMP ($30FF): low byte from $30FF, high byte from $3000 — not $3100.
        let (mut c, mut b) = setup(&[0xFF, 0x30]);
        b.mem[0x30FF] = 0xCD;
        b.mem[0x3000] = 0xAB; // what the hardware actually reads
        b.mem[0x3100] = 0x99; // what a naive implementation would read
        let op = c.resolve(&mut b, Mode::Indirect).unwrap();
        assert_eq!(op.addr, 0xABCD, "the page-boundary bug is real behaviour");
    }

    #[test]
    fn indirect_vectors_wrap_inside_page_zero() {
        // ($FF,X) with X=0: low byte from $FF, high byte from $00.
        let (mut c, mut b) = setup(&[0xFF]);
        c.x = 0;
        b.mem[0x00FF] = 0x34;
        b.mem[0x0000] = 0x12;
        b.mem[0x0100] = 0x99; // must not be consulted
        assert_eq!(c.resolve(&mut b, Mode::IndirectX).unwrap().addr, 0x1234);

        let (mut c, mut b) = setup(&[0xFF]);
        c.y = 0;
        b.mem[0x00FF] = 0x34;
        b.mem[0x0000] = 0x12;
        assert_eq!(c.resolve(&mut b, Mode::IndirectY).unwrap().addr, 0x1234);
    }

    #[test]
    fn indirect_x_adds_before_dereferencing() {
        // ($20,X) with X=$05 dereferences $25, not $20.
        let (mut c, mut b) = setup(&[0x20]);
        c.x = 0x05;
        b.mem[0x0025] = 0x00;
        b.mem[0x0026] = 0x80;
        assert_eq!(c.resolve(&mut b, Mode::IndirectX).unwrap().addr, 0x8000);
    }

    #[test]
    fn indirect_y_adds_after_dereferencing() {
        // ($20),Y with Y=$05 dereferences $20 then adds 5.
        let (mut c, mut b) = setup(&[0x20]);
        c.y = 0x05;
        b.mem[0x0020] = 0x00;
        b.mem[0x0021] = 0x80;
        let op = c.resolve(&mut b, Mode::IndirectY).unwrap();
        assert_eq!(op.addr, 0x8005);
        assert!(!op.page_crossed);
    }

    #[test]
    fn page_crossings_are_reported_for_indexed_reads() {
        // $80FF + 1 crosses into $8100.
        let (mut c, mut b) = setup(&[0xFF, 0x80]);
        c.x = 0x01;
        let op = c.resolve(&mut b, Mode::AbsoluteX).unwrap();
        assert_eq!(op.addr, 0x8100);
        assert!(op.page_crossed, "costs an extra read cycle");

        let (mut c, mut b) = setup(&[0x00, 0x80]);
        c.x = 0x01;
        assert!(!c.resolve(&mut b, Mode::AbsoluteX).unwrap().page_crossed);

        // ($20),Y crossing
        let (mut c, mut b) = setup(&[0x20]);
        c.y = 0x02;
        b.mem[0x0020] = 0xFF;
        b.mem[0x0021] = 0x80;
        let op = c.resolve(&mut b, Mode::IndirectY).unwrap();
        assert_eq!(op.addr, 0x8101);
        assert!(op.page_crossed);
    }

    #[test]
    fn relative_displacement_is_signed_and_from_the_next_instruction() {
        // Backwards: -3 from the byte after the displacement.
        let (mut c, mut b) = setup(&[0xFD]);
        let op = c.resolve(&mut b, Mode::Relative).unwrap();
        assert_eq!(op.addr, 0x0201u16.wrapping_sub(3));

        // Forwards.
        let (mut c, mut b) = setup(&[0x05]);
        assert_eq!(c.resolve(&mut b, Mode::Relative).unwrap().addr, 0x0206);
    }

    #[test]
    fn immediate_points_at_the_operand_byte_and_advances_pc() {
        let (mut c, mut b) = setup(&[0x42]);
        let op = c.resolve(&mut b, Mode::Immediate).unwrap();
        assert_eq!(op.addr, 0x0200);
        assert_eq!(c.pc, 0x0201);
        assert_eq!(b.read(op.addr), 0x42);
    }

    #[test]
    fn implied_and_accumulator_have_no_address() {
        let (mut c, mut b) = setup(&[]);
        assert!(c.resolve(&mut b, Mode::Implied).is_none());
        assert!(c.resolve(&mut b, Mode::Accumulator).is_none());
        assert_eq!(c.pc, 0x0200, "neither consumes an operand byte");
    }
}
