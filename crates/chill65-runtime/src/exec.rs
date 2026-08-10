//! Instruction decode and execution.
//!
//! One `step()` executes one instruction and returns the cycles it took, which
//! the caller feeds to [`Bus::tick`]. Everything clocked on this board — the
//! video counters that raise IRQ, the watchdog, the POKEY LFSR — runs off that
//! count, so nothing depends on host time.
//!
//! This task covers addressing, loads and stores, transfers, the stack, flag
//! operations and the bitwise group. Arithmetic, shifts, compares and control
//! flow arrive next; interrupts and the page-cross/branch cycle penalties after
//! that. Opcodes not yet implemented return [`CpuError::IllegalOpcode`] rather
//! than doing something plausible, so a gap is loud.

use crate::addr::Mode;
use crate::bus::Bus;
use crate::cpu::{flag, Cpu, CpuError};

/// Where the stack lives. `S` is the low byte of an address in page one.
const STACK_BASE: u16 = 0x0100;

impl Cpu {
    pub fn push(&mut self, bus: &mut impl Bus, v: u8) {
        bus.write(STACK_BASE | self.s as u16, v);
        self.s = self.s.wrapping_sub(1);
    }

    pub fn pull(&mut self, bus: &mut impl Bus) -> u8 {
        self.s = self.s.wrapping_add(1);
        bus.read(STACK_BASE | self.s as u16)
    }

    /// Execute one instruction. Returns the cycles consumed.
    ///
    /// Base cycle counts are recorded here as each instruction is implemented;
    /// the page-cross and branch penalties are layered on in a later task.
    pub fn step(&mut self, bus: &mut impl Bus) -> Result<u8, CpuError> {
        let pc = self.pc;
        bus.begin_instruction(pc);
        let opcode = self.fetch(bus);

        let cycles = match opcode {
            // ---- LDA ----
            0xA9 => self.load_a(bus, Mode::Immediate, 2),
            0xA5 => self.load_a(bus, Mode::ZeroPage, 3),
            0xB5 => self.load_a(bus, Mode::ZeroPageX, 4),
            0xAD => self.load_a(bus, Mode::Absolute, 4),
            0xBD => self.load_a(bus, Mode::AbsoluteX, 4),
            0xB9 => self.load_a(bus, Mode::AbsoluteY, 4),
            0xA1 => self.load_a(bus, Mode::IndirectX, 6),
            0xB1 => self.load_a(bus, Mode::IndirectY, 5),

            // ---- LDX ----
            0xA2 => self.load_x(bus, Mode::Immediate, 2),
            0xA6 => self.load_x(bus, Mode::ZeroPage, 3),
            0xB6 => self.load_x(bus, Mode::ZeroPageY, 4),
            0xAE => self.load_x(bus, Mode::Absolute, 4),
            0xBE => self.load_x(bus, Mode::AbsoluteY, 4),

            // ---- LDY ----
            0xA0 => self.load_y(bus, Mode::Immediate, 2),
            0xA4 => self.load_y(bus, Mode::ZeroPage, 3),
            0xB4 => self.load_y(bus, Mode::ZeroPageX, 4),
            0xAC => self.load_y(bus, Mode::Absolute, 4),
            0xBC => self.load_y(bus, Mode::AbsoluteX, 4),

            // ---- STA ---- (no immediate form; PST65 spells it MOST-I)
            0x85 => self.store(bus, Mode::ZeroPage, self.a, 3),
            0x95 => self.store(bus, Mode::ZeroPageX, self.a, 4),
            0x8D => self.store(bus, Mode::Absolute, self.a, 4),
            0x9D => self.store(bus, Mode::AbsoluteX, self.a, 5),
            0x99 => self.store(bus, Mode::AbsoluteY, self.a, 5),
            0x81 => self.store(bus, Mode::IndirectX, self.a, 6),
            0x91 => self.store(bus, Mode::IndirectY, self.a, 6),

            // ---- STX / STY ----
            0x86 => self.store(bus, Mode::ZeroPage, self.x, 3),
            0x96 => self.store(bus, Mode::ZeroPageY, self.x, 4),
            0x8E => self.store(bus, Mode::Absolute, self.x, 4),
            0x84 => self.store(bus, Mode::ZeroPage, self.y, 3),
            0x94 => self.store(bus, Mode::ZeroPageX, self.y, 4),
            0x8C => self.store(bus, Mode::Absolute, self.y, 4),

            // ---- transfers ----
            0xAA => {
                self.x = self.a;
                self.set_nz(self.x);
                2
            }
            0xA8 => {
                self.y = self.a;
                self.set_nz(self.y);
                2
            }
            0x8A => {
                self.a = self.x;
                self.set_nz(self.a);
                2
            }
            0x98 => {
                self.a = self.y;
                self.set_nz(self.a);
                2
            }
            0xBA => {
                self.x = self.s;
                self.set_nz(self.x);
                2
            }
            // TXS is the one transfer that does NOT touch the flags.
            0x9A => {
                self.s = self.x;
                2
            }

            // ---- stack ----
            0x48 => {
                self.push(bus, self.a);
                3
            }
            0x68 => {
                self.a = self.pull(bus);
                self.set_nz(self.a);
                4
            }
            // PHP pushes with B set — B exists only in pushed copies.
            0x08 => {
                let p = self.p() | flag::BREAK;
                self.push(bus, p);
                3
            }
            0x28 => {
                let p = self.pull(bus);
                self.set_p(p);
                4
            }

            // ---- flags ----
            0x18 => {
                self.carry = false;
                2
            }
            0x38 => {
                self.carry = true;
                2
            }
            0x58 => {
                self.interrupt_disable = false;
                2
            }
            0x78 => {
                self.interrupt_disable = true;
                2
            }
            0xD8 => {
                self.decimal = false;
                2
            }
            0xF8 => {
                self.decimal = true;
                2
            }
            0xB8 => {
                self.overflow = false;
                2
            }

            // ---- bitwise ----
            0x29 => self.and(bus, Mode::Immediate, 2),
            0x25 => self.and(bus, Mode::ZeroPage, 3),
            0x35 => self.and(bus, Mode::ZeroPageX, 4),
            0x2D => self.and(bus, Mode::Absolute, 4),
            0x3D => self.and(bus, Mode::AbsoluteX, 4),
            0x39 => self.and(bus, Mode::AbsoluteY, 4),
            0x21 => self.and(bus, Mode::IndirectX, 6),
            0x31 => self.and(bus, Mode::IndirectY, 5),

            0x09 => self.ora(bus, Mode::Immediate, 2),
            0x05 => self.ora(bus, Mode::ZeroPage, 3),
            0x15 => self.ora(bus, Mode::ZeroPageX, 4),
            0x0D => self.ora(bus, Mode::Absolute, 4),
            0x1D => self.ora(bus, Mode::AbsoluteX, 4),
            0x19 => self.ora(bus, Mode::AbsoluteY, 4),
            0x01 => self.ora(bus, Mode::IndirectX, 6),
            0x11 => self.ora(bus, Mode::IndirectY, 5),

            0x49 => self.eor(bus, Mode::Immediate, 2),
            0x45 => self.eor(bus, Mode::ZeroPage, 3),
            0x55 => self.eor(bus, Mode::ZeroPageX, 4),
            0x4D => self.eor(bus, Mode::Absolute, 4),
            0x5D => self.eor(bus, Mode::AbsoluteX, 4),
            0x59 => self.eor(bus, Mode::AbsoluteY, 4),
            0x41 => self.eor(bus, Mode::IndirectX, 6),
            0x51 => self.eor(bus, Mode::IndirectY, 5),

            // BIT sets N and V from the *memory* byte's top two bits, and Z
            // from the AND — the accumulator is never modified.
            0x24 => self.bit(bus, Mode::ZeroPage, 3),
            0x2C => self.bit(bus, Mode::Absolute, 4),

            // ---- ADC / SBC ----
            0x69 => self.adc(bus, Mode::Immediate, 2),
            0x65 => self.adc(bus, Mode::ZeroPage, 3),
            0x75 => self.adc(bus, Mode::ZeroPageX, 4),
            0x6D => self.adc(bus, Mode::Absolute, 4),
            0x7D => self.adc(bus, Mode::AbsoluteX, 4),
            0x79 => self.adc(bus, Mode::AbsoluteY, 4),
            0x61 => self.adc(bus, Mode::IndirectX, 6),
            0x71 => self.adc(bus, Mode::IndirectY, 5),

            0xE9 => self.sbc(bus, Mode::Immediate, 2),
            0xE5 => self.sbc(bus, Mode::ZeroPage, 3),
            0xF5 => self.sbc(bus, Mode::ZeroPageX, 4),
            0xED => self.sbc(bus, Mode::Absolute, 4),
            0xFD => self.sbc(bus, Mode::AbsoluteX, 4),
            0xF9 => self.sbc(bus, Mode::AbsoluteY, 4),
            0xE1 => self.sbc(bus, Mode::IndirectX, 6),
            0xF1 => self.sbc(bus, Mode::IndirectY, 5),

            // ---- compares ----
            0xC9 => self.compare(bus, Mode::Immediate, self.a, 2),
            0xC5 => self.compare(bus, Mode::ZeroPage, self.a, 3),
            0xD5 => self.compare(bus, Mode::ZeroPageX, self.a, 4),
            0xCD => self.compare(bus, Mode::Absolute, self.a, 4),
            0xDD => self.compare(bus, Mode::AbsoluteX, self.a, 4),
            0xD9 => self.compare(bus, Mode::AbsoluteY, self.a, 4),
            0xC1 => self.compare(bus, Mode::IndirectX, self.a, 6),
            0xD1 => self.compare(bus, Mode::IndirectY, self.a, 5),
            0xE0 => self.compare(bus, Mode::Immediate, self.x, 2),
            0xE4 => self.compare(bus, Mode::ZeroPage, self.x, 3),
            0xEC => self.compare(bus, Mode::Absolute, self.x, 4),
            0xC0 => self.compare(bus, Mode::Immediate, self.y, 2),
            0xC4 => self.compare(bus, Mode::ZeroPage, self.y, 3),
            0xCC => self.compare(bus, Mode::Absolute, self.y, 4),

            // ---- increment / decrement ----
            0xE6 => self.rmw(bus, Mode::ZeroPage, Op::Inc, 5),
            0xF6 => self.rmw(bus, Mode::ZeroPageX, Op::Inc, 6),
            0xEE => self.rmw(bus, Mode::Absolute, Op::Inc, 6),
            0xFE => self.rmw(bus, Mode::AbsoluteX, Op::Inc, 7),
            0xC6 => self.rmw(bus, Mode::ZeroPage, Op::Dec, 5),
            0xD6 => self.rmw(bus, Mode::ZeroPageX, Op::Dec, 6),
            0xCE => self.rmw(bus, Mode::Absolute, Op::Dec, 6),
            0xDE => self.rmw(bus, Mode::AbsoluteX, Op::Dec, 7),

            0xE8 => {
                self.x = self.x.wrapping_add(1);
                self.set_nz(self.x);
                2
            }
            0xC8 => {
                self.y = self.y.wrapping_add(1);
                self.set_nz(self.y);
                2
            }
            0xCA => {
                self.x = self.x.wrapping_sub(1);
                self.set_nz(self.x);
                2
            }
            0x88 => {
                self.y = self.y.wrapping_sub(1);
                self.set_nz(self.y);
                2
            }

            // ---- shifts and rotates ----
            0x0A => self.shift_a(Op::Asl),
            0x4A => self.shift_a(Op::Lsr),
            0x2A => self.shift_a(Op::Rol),
            0x6A => self.shift_a(Op::Ror),

            0x06 => self.rmw(bus, Mode::ZeroPage, Op::Asl, 5),
            0x16 => self.rmw(bus, Mode::ZeroPageX, Op::Asl, 6),
            0x0E => self.rmw(bus, Mode::Absolute, Op::Asl, 6),
            0x1E => self.rmw(bus, Mode::AbsoluteX, Op::Asl, 7),
            0x46 => self.rmw(bus, Mode::ZeroPage, Op::Lsr, 5),
            0x56 => self.rmw(bus, Mode::ZeroPageX, Op::Lsr, 6),
            0x4E => self.rmw(bus, Mode::Absolute, Op::Lsr, 6),
            0x5E => self.rmw(bus, Mode::AbsoluteX, Op::Lsr, 7),
            0x26 => self.rmw(bus, Mode::ZeroPage, Op::Rol, 5),
            0x36 => self.rmw(bus, Mode::ZeroPageX, Op::Rol, 6),
            0x2E => self.rmw(bus, Mode::Absolute, Op::Rol, 6),
            0x3E => self.rmw(bus, Mode::AbsoluteX, Op::Rol, 7),
            0x66 => self.rmw(bus, Mode::ZeroPage, Op::Ror, 5),
            0x76 => self.rmw(bus, Mode::ZeroPageX, Op::Ror, 6),
            0x6E => self.rmw(bus, Mode::Absolute, Op::Ror, 6),
            0x7E => self.rmw(bus, Mode::AbsoluteX, Op::Ror, 7),

            // ---- branches ----
            0x90 => self.branch(bus, !self.carry),
            0xB0 => self.branch(bus, self.carry),
            0xD0 => self.branch(bus, !self.zero),
            0xF0 => self.branch(bus, self.zero),
            0x10 => self.branch(bus, !self.negative),
            0x30 => self.branch(bus, self.negative),
            0x50 => self.branch(bus, !self.overflow),
            0x70 => self.branch(bus, self.overflow),

            // ---- jumps and subroutines ----
            0x4C => {
                self.pc = self.resolve(bus, Mode::Absolute).unwrap().addr;
                3
            }
            0x6C => {
                self.pc = self.resolve(bus, Mode::Indirect).unwrap().addr;
                5
            }
            0x20 => {
                let target = self.fetch_u16(bus);
                // Pushes the address of the LAST byte of the JSR, not the
                // return address — RTS adds one back.
                let ret = self.pc.wrapping_sub(1);
                self.push(bus, (ret >> 8) as u8);
                self.push(bus, ret as u8);
                self.pc = target;
                6
            }
            0x60 => {
                let lo = self.pull(bus) as u16;
                let hi = self.pull(bus) as u16;
                self.pc = ((hi << 8) | lo).wrapping_add(1);
                6
            }

            // BRK is two bytes: the second is padding the processor skips.
            0x00 => {
                self.pc = self.pc.wrapping_add(1);
                self.enter_interrupt(bus, crate::cpu::vector::IRQ, true);
                7
            }
            // RTI pulls PC exactly — unlike RTS it does not add one.
            0x40 => {
                let p = self.pull(bus);
                self.set_p(p);
                let lo = self.pull(bus) as u16;
                let hi = self.pull(bus) as u16;
                self.pc = (hi << 8) | lo;
                6
            }

            0xEA => 2, // NOP

            _ => return Err(CpuError::IllegalOpcode { opcode, pc }),
        };

        self.cycles += cycles as u64;
        bus.tick(cycles);
        Ok(cycles)
    }

    /// Read an operand, reporting whether reaching it crossed a page.
    ///
    /// Indexed **reads** cost an extra cycle on a crossing because the hardware
    /// speculatively reads the wrong page first and re-reads. Indexed *writes*
    /// do not: they always pay the extra cycle, which is why `STA abs,X` is a
    /// flat 5 and needs no penalty.
    fn operand_value(&mut self, bus: &mut impl Bus, mode: Mode) -> (u8, bool) {
        let op = self.resolve(bus, mode).expect("mode has an address");
        (bus.read(op.addr), op.page_crossed)
    }

    fn load_a(&mut self, bus: &mut impl Bus, mode: Mode, c: u8) -> u8 {
        let (v, crossed) = self.operand_value(bus, mode);
        self.a = v;
        self.set_nz(v);
        c + crossed as u8
    }

    fn load_x(&mut self, bus: &mut impl Bus, mode: Mode, c: u8) -> u8 {
        let (v, crossed) = self.operand_value(bus, mode);
        self.x = v;
        self.set_nz(v);
        c + crossed as u8
    }

    fn load_y(&mut self, bus: &mut impl Bus, mode: Mode, c: u8) -> u8 {
        let (v, crossed) = self.operand_value(bus, mode);
        self.y = v;
        self.set_nz(v);
        c + crossed as u8
    }

    fn store(&mut self, bus: &mut impl Bus, mode: Mode, v: u8, c: u8) -> u8 {
        let op = self.resolve(bus, mode).expect("mode has an address");
        bus.write(op.addr, v);
        c
    }

    fn and(&mut self, bus: &mut impl Bus, mode: Mode, c: u8) -> u8 {
        let (v, crossed) = self.operand_value(bus, mode);
        self.a &= v;
        self.set_nz(self.a);
        c + crossed as u8
    }

    fn ora(&mut self, bus: &mut impl Bus, mode: Mode, c: u8) -> u8 {
        let (v, crossed) = self.operand_value(bus, mode);
        self.a |= v;
        self.set_nz(self.a);
        c + crossed as u8
    }

    fn eor(&mut self, bus: &mut impl Bus, mode: Mode, c: u8) -> u8 {
        let (v, crossed) = self.operand_value(bus, mode);
        self.a ^= v;
        self.set_nz(self.a);
        c + crossed as u8
    }

    fn bit(&mut self, bus: &mut impl Bus, mode: Mode, c: u8) -> u8 {
        let (v, _) = self.operand_value(bus, mode);
        self.zero = (self.a & v) == 0;
        self.negative = v & 0x80 != 0;
        self.overflow = v & 0x40 != 0;
        c
    }
}


impl Cpu {
    /// Push PC and P, mask further IRQs, and vector.
    ///
    /// `from_brk` decides the B flag in the *pushed* copy of P — set for
    /// `BRK`/`PHP`, clear for a hardware interrupt. It is the only way an
    /// interrupt handler can tell the two apart, since B has no home in the
    /// processor itself.
    ///
    /// Note what is **not** done here: D is left alone. The NMOS 6502 does not
    /// clear decimal mode on interrupt entry, which is exactly why
    /// `CIN.MAC:20` opens the game's ISR with `CLD ; set to binary mode`.
    /// Clearing it here would make that instruction redundant and would mask a
    /// real difference from the 65C02.
    pub fn enter_interrupt(&mut self, bus: &mut impl Bus, vector: u16, from_brk: bool) {
        self.push(bus, (self.pc >> 8) as u8);
        self.push(bus, self.pc as u8);
        let mut p = self.p();
        if from_brk {
            p |= flag::BREAK;
        }
        self.push(bus, p);
        self.interrupt_disable = true;
        self.pc = bus.read_u16(vector);
    }

    /// Deliver a maskable interrupt. Returns the cycles taken, or 0 if masked.
    ///
    /// The board raises this four times a frame from the video counters
    /// (`SyncChain.v:45`, `IRQCLK = ~vcount[5]`), and the handler acknowledges
    /// it by strobing INTACK.
    pub fn irq(&mut self, bus: &mut impl Bus) -> u8 {
        if self.interrupt_disable {
            return 0;
        }
        self.enter_interrupt(bus, crate::cpu::vector::IRQ, false);
        self.cycles += 7;
        bus.tick(7);
        7
    }

    /// Deliver a non-maskable interrupt.
    ///
    /// Implemented for completeness and testability, but **this board can never
    /// raise it**: `MicroProcessor.v:26` ties the line low with `.NMI(1'b0)`.
    /// The vector at `FFFA` exists and points at `E009`, and is never taken.
    pub fn nmi(&mut self, bus: &mut impl Bus) -> u8 {
        self.enter_interrupt(bus, crate::cpu::vector::NMI, false);
        self.cycles += 7;
        bus.tick(7);
        7
    }
}

/// Read-modify-write and accumulator operations that share flag handling.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Inc,
    Dec,
    Asl,
    Lsr,
    Rol,
    Ror,
}

impl Cpu {
    fn compare(&mut self, bus: &mut impl Bus, mode: Mode, reg: u8, c: u8) -> u8 {
        let (v, crossed) = self.operand_value(bus, mode);
        let r = reg.wrapping_sub(v);
        // Carry is an unsigned "greater or equal"; there is no overflow flag.
        self.carry = reg >= v;
        self.set_nz(r);
        c + crossed as u8
    }

    fn apply_op(&mut self, op: Op, v: u8) -> u8 {
        match op {
            Op::Inc => v.wrapping_add(1),
            Op::Dec => v.wrapping_sub(1),
            Op::Asl => {
                self.carry = v & 0x80 != 0;
                v << 1
            }
            Op::Lsr => {
                self.carry = v & 0x01 != 0;
                v >> 1
            }
            Op::Rol => {
                let old = self.carry as u8;
                self.carry = v & 0x80 != 0;
                (v << 1) | old
            }
            Op::Ror => {
                let old = (self.carry as u8) << 7;
                self.carry = v & 0x01 != 0;
                (v >> 1) | old
            }
        }
    }

    fn rmw(&mut self, bus: &mut impl Bus, mode: Mode, op: Op, c: u8) -> u8 {
        let addr = self.resolve(bus, mode).expect("mode has an address").addr;
        let v = bus.read(addr);
        let r = self.apply_op(op, v);
        bus.write(addr, r);
        self.set_nz(r);
        c
    }

    fn shift_a(&mut self, op: Op) -> u8 {
        self.a = self.apply_op(op, self.a);
        self.set_nz(self.a);
        2
    }

    /// Branches cost 2, +1 when taken, +1 more when the target is on another
    /// page. HLL65F's structured control flow emits one of these per construct,
    /// so their timing is most of the game's timing.
    fn branch(&mut self, bus: &mut impl Bus, take: bool) -> u8 {
        let op = self.resolve(bus, Mode::Relative).expect("relative");
        if !take {
            return 2;
        }
        self.pc = op.addr;
        if op.page_crossed {
            4
        } else {
            3
        }
    }

    /// Add with carry.
    ///
    /// Decimal mode is NMOS, and its flags are the trap: **N, V and Z do not
    /// describe the decimal result**. Z comes from the binary sum, and N and V
    /// from the intermediate before the high-nibble correction. The 65C02 fixed
    /// this; the 6502 in a 1983 coin-op did not, and `CCN.MAC`'s coin counting
    /// and `CAL.MAC`'s scoring both run with D set.
    fn adc(&mut self, bus: &mut impl Bus, mode: Mode, c: u8) -> u8 {
        let (m, crossed) = self.operand_value(bus, mode);
        let c = c + crossed as u8;
        let a = self.a;
        let carry_in = self.carry as u16;

        if !self.decimal {
            let sum = a as u16 + m as u16 + carry_in;
            let r = sum as u8;
            self.carry = sum > 0xFF;
            // Overflow: both inputs agreed in sign and the result disagrees.
            self.overflow = ((a ^ r) & (m ^ r) & 0x80) != 0;
            self.a = r;
            self.set_nz(r);
            return c;
        }

        let binary = (a as u16).wrapping_add(m as u16).wrapping_add(carry_in) as u8;
        let mut lo = (a & 0x0F) as u16 + (m & 0x0F) as u16 + carry_in;
        if lo > 9 {
            lo += 6;
        }
        let mut hi = (a >> 4) as u16 + (m >> 4) as u16 + if lo > 0x0F { 1 } else { 0 };

        self.zero = binary == 0;
        self.negative = (hi & 0x08) != 0;
        self.overflow = ((a ^ m) & 0x80) == 0 && ((a as u16 ^ (hi << 4)) & 0x80) != 0;

        if hi > 9 {
            hi += 6;
        }
        self.carry = hi > 0x0F;
        self.a = (((hi << 4) | (lo & 0x0F)) & 0xFF) as u8;
        c
    }

    /// Subtract with borrow.
    ///
    /// On NMOS **every** flag comes from the binary operation even in decimal
    /// mode — only the accumulator differs. That asymmetry with ADC is real
    /// hardware behaviour, not an oversight here.
    fn sbc(&mut self, bus: &mut impl Bus, mode: Mode, c: u8) -> u8 {
        let (m, crossed) = self.operand_value(bus, mode);
        let c = c + crossed as u8;
        let a = self.a;
        let borrow = 1 - self.carry as u16;

        let diff = (a as u16).wrapping_sub(m as u16).wrapping_sub(borrow);
        let r = diff as u8;
        self.carry = diff < 0x100;
        self.overflow = ((a ^ m) & (a ^ r) & 0x80) != 0;
        self.set_nz(r);

        if !self.decimal {
            self.a = r;
            return c;
        }

        let mut lo = (a & 0x0F) as i16 - (m & 0x0F) as i16 - borrow as i16;
        let mut hi = (a >> 4) as i16 - (m >> 4) as i16;
        if lo & 0x10 != 0 {
            lo -= 6;
            hi -= 1;
        }
        if hi & 0x10 != 0 {
            hi -= 6;
        }
        self.a = (((hi as u8) << 4) | (lo as u8 & 0x0F)) as u8;
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::FlatBus;

    fn run(prog: &[u8]) -> (Cpu, FlatBus) {
        let mut bus = FlatBus::new();
        bus.load(0x0200, prog);
        let mut cpu = Cpu::new();
        cpu.pc = 0x0200;
        (cpu, bus)
    }

    fn step_n(cpu: &mut Cpu, bus: &mut FlatBus, n: usize) {
        for _ in 0..n {
            cpu.step(bus).expect("step failed");
        }
    }

    #[test]
    fn lda_immediate_sets_negative_and_zero() {
        let (mut c, mut b) = run(&[0xA9, 0x80]);
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x80);
        assert!(c.negative && !c.zero);

        let (mut c, mut b) = run(&[0xA9, 0x00]);
        c.step(&mut b).unwrap();
        assert!(c.zero && !c.negative);

        let (mut c, mut b) = run(&[0xA9, 0x01]);
        c.step(&mut b).unwrap();
        assert!(!c.zero && !c.negative);
    }

    #[test]
    fn the_real_reset_sequence_executes() {
        // E000 in the shipped ROM: a9 00 8d 87 9e — LDA #0 / STA $9E87.
        // That is CIN.MAC:7 `TRAI 0 HW.BSL`, the first thing the board does.
        let (mut c, mut b) = run(&[0xA9, 0x00, 0x8D, 0x87, 0x9E]);
        step_n(&mut c, &mut b, 2);
        assert_eq!(c.a, 0x00);
        assert_eq!(b.mem[0x9E87], 0x00, "bank select written");
        assert_eq!(c.cycles, 2 + 4);
    }

    #[test]
    fn the_isr_prologue_executes() {
        // E009: 48 8a 48 — PHA / TXA / PHA, CIN.MAC's register save.
        let (mut c, mut b) = run(&[0x48, 0x8A, 0x48]);
        c.a = 0x11;
        c.x = 0x22;
        step_n(&mut c, &mut b, 3);
        assert_eq!(c.s, 0xFB, "two bytes pushed");
        assert_eq!(b.mem[0x01FD], 0x11);
        assert_eq!(b.mem[0x01FC], 0x22);
        assert_eq!(c.a, 0x22, "TXA moved X into A");
    }

    #[test]
    fn stores_reach_every_addressing_mode() {
        // STA $9E87 absolute — the bank-select latch.
        let (mut c, mut b) = run(&[0x8D, 0x87, 0x9E]);
        c.a = 0xFF;
        c.step(&mut b).unwrap();
        assert_eq!(b.mem[0x9E87], 0xFF);

        // STA ($20),Y
        let (mut c, mut b) = run(&[0x91, 0x20]);
        c.a = 0x5A;
        c.y = 0x02;
        b.mem[0x0020] = 0x00;
        b.mem[0x0021] = 0x80;
        c.step(&mut b).unwrap();
        assert_eq!(b.mem[0x8002], 0x5A);

        // STX $10,Y — the only zero-page,Y store
        let (mut c, mut b) = run(&[0x96, 0x10]);
        c.x = 0x77;
        c.y = 0x05;
        c.step(&mut b).unwrap();
        assert_eq!(b.mem[0x0015], 0x77);
    }

    #[test]
    fn txs_does_not_touch_the_flags() {
        // Every other transfer sets N and Z; TXS does not, because it targets
        // the stack pointer. Getting this wrong corrupts flags at every stack
        // setup — and the board's reset does `LDX #0FF / TXS`.
        let (mut c, mut b) = run(&[0x9A]);
        c.x = 0x00;
        c.zero = false;
        c.negative = true;
        c.step(&mut b).unwrap();
        assert_eq!(c.s, 0x00);
        assert!(!c.zero, "TXS must not set Z");
        assert!(c.negative, "TXS must not clear N");

        // TSX, by contrast, does.
        let (mut c, mut b) = run(&[0xBA]);
        c.s = 0x00;
        c.step(&mut b).unwrap();
        assert!(c.zero);
    }

    #[test]
    fn php_pushes_break_set_but_does_not_store_it() {
        let (mut c, mut b) = run(&[0x08]);
        c.carry = true;
        c.step(&mut b).unwrap();
        let pushed = b.mem[0x01FD];
        assert_eq!(pushed & flag::BREAK, flag::BREAK, "B set in the pushed copy");
        assert_eq!(pushed & flag::UNUSED, flag::UNUSED);
        assert_eq!(pushed & flag::CARRY, flag::CARRY);
        assert_eq!(c.p() & flag::BREAK, 0, "still not in the processor");
    }

    #[test]
    fn plp_restores_flags_and_ignores_break() {
        let (mut c, mut b) = run(&[0x28]);
        b.mem[0x01FE] = flag::CARRY | flag::NEGATIVE | flag::BREAK;
        c.s = 0xFD;
        c.step(&mut b).unwrap();
        assert!(c.carry && c.negative);
        assert_eq!(c.p() & flag::BREAK, 0);
    }

    #[test]
    fn stack_wraps_within_page_one() {
        let (mut c, mut b) = run(&[0x48, 0x48]);
        c.s = 0x00;
        c.a = 0xAA;
        step_n(&mut c, &mut b, 2);
        assert_eq!(b.mem[0x0100], 0xAA);
        assert_eq!(b.mem[0x01FF], 0xAA, "wrapped to the top of page one");
        assert_eq!(c.s, 0xFE);
    }

    #[test]
    fn flag_instructions() {
        for (op, get, want) in [
            (0x18u8, "carry", false),
            (0x38, "carry", true),
            (0x58, "interrupt", false),
            (0x78, "interrupt", true),
            (0xD8, "decimal", false),
            (0xF8, "decimal", true),
        ] {
            let (mut c, mut b) = run(&[op]);
            c.carry = !want;
            c.interrupt_disable = !want;
            c.decimal = !want;
            c.step(&mut b).unwrap();
            let got = match get {
                "carry" => c.carry,
                "interrupt" => c.interrupt_disable,
                _ => c.decimal,
            };
            assert_eq!(got, want, "opcode {op:#04x}");
        }
        // CLV only clears; there is no SEV.
        let (mut c, mut b) = run(&[0xB8]);
        c.overflow = true;
        c.step(&mut b).unwrap();
        assert!(!c.overflow);
    }

    #[test]
    fn bitwise_group() {
        let (mut c, mut b) = run(&[0x29, 0x0F]);
        c.a = 0xFF;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x0F);

        let (mut c, mut b) = run(&[0x09, 0xF0]);
        c.a = 0x0F;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0xFF);
        assert!(c.negative);

        let (mut c, mut b) = run(&[0x49, 0xFF]);
        c.a = 0xFF;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x00);
        assert!(c.zero);
    }

    #[test]
    fn bit_takes_n_and_v_from_memory_not_the_result() {
        // BIT $10 where memory is $C0: N and V come from bits 7 and 6 of the
        // *memory* byte, Z from A AND memory, and A is untouched.
        let (mut c, mut b) = run(&[0x24, 0x10]);
        b.mem[0x0010] = 0xC0;
        c.a = 0x01;
        c.step(&mut b).unwrap();
        assert!(c.negative, "N from memory bit 7");
        assert!(c.overflow, "V from memory bit 6");
        assert!(c.zero, "A & M is zero");
        assert_eq!(c.a, 0x01, "BIT never modifies the accumulator");

        // A non-zero AND clears Z but leaves N and V from memory.
        let (mut c, mut b) = run(&[0x2C, 0x10, 0x00]);
        b.mem[0x0010] = 0xC1;
        c.a = 0x01;
        c.step(&mut b).unwrap();
        assert!(!c.zero);
        assert!(c.negative && c.overflow);
    }

    #[test]
    fn illegal_opcodes_error_rather_than_panic() {
        // A ROM image is input; input must not be able to take the process down.
        let (mut c, mut b) = run(&[0xFF]);
        match c.step(&mut b) {
            Err(CpuError::IllegalOpcode { opcode, pc }) => {
                assert_eq!(opcode, 0xFF);
                assert_eq!(pc, 0x0200, "reports where it was, not where PC ended");
            }
            other => panic!("expected IllegalOpcode, got {other:?}"),
        }
    }

    #[test]
    fn cycles_accumulate_on_the_cpu_and_the_bus_together() {
        let (mut c, mut b) = run(&[0xA9, 0x01, 0xA5, 0x10]);
        step_n(&mut c, &mut b, 2);
        assert_eq!(c.cycles, 2 + 3);
        assert_eq!(b.cycles, c.cycles, "the bus clocks from the same count");
    }

    #[test]
    fn adc_binary_carry_and_overflow() {
        // 0x50 + 0x50 = 0xA0: two positives make a negative — V set.
        let (mut c, mut b) = run(&[0x69, 0x50]);
        c.a = 0x50;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0xA0);
        assert!(c.overflow, "positive + positive -> negative");
        assert!(c.negative);
        assert!(!c.carry);

        // 0xD0 + 0x90 = 0x60 with carry: two negatives make a positive — V set.
        let (mut c, mut b) = run(&[0x69, 0x90]);
        c.a = 0xD0;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x60);
        assert!(c.overflow, "negative + negative -> positive");
        assert!(c.carry);

        // Mixed signs can never overflow.
        let (mut c, mut b) = run(&[0x69, 0xF0]);
        c.a = 0x10;
        c.step(&mut b).unwrap();
        assert!(!c.overflow, "mixed signs cannot overflow");

        // Carry in is added.
        let (mut c, mut b) = run(&[0x69, 0x01]);
        c.a = 0x01;
        c.carry = true;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x03);
    }

    #[test]
    fn sbc_borrow_and_overflow() {
        // Carry set means "no borrow": 0x50 - 0x10 = 0x40.
        let (mut c, mut b) = run(&[0xE9, 0x10]);
        c.a = 0x50;
        c.carry = true;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x40);
        assert!(c.carry, "no borrow occurred");

        // Carry clear subtracts one more.
        let (mut c, mut b) = run(&[0xE9, 0x10]);
        c.a = 0x50;
        c.carry = false;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x3F);

        // 0x50 - 0xB0 = 0xA0: positive minus negative giving negative — V set.
        let (mut c, mut b) = run(&[0xE9, 0xB0]);
        c.a = 0x50;
        c.carry = true;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0xA0);
        assert!(c.overflow);
        assert!(!c.carry, "borrow occurred");
    }

    #[test]
    fn adc_decimal_mode() {
        // Ordinary BCD: 12 + 34 = 46.
        let (mut c, mut b) = run(&[0x69, 0x34]);
        c.decimal = true;
        c.a = 0x12;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x46);
        assert!(!c.carry);

        // Low-nibble carry: 15 + 26 = 41.
        let (mut c, mut b) = run(&[0x69, 0x26]);
        c.decimal = true;
        c.a = 0x15;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x41);

        // Wrap past 99: 99 + 01 = 00 with carry.
        let (mut c, mut b) = run(&[0x69, 0x01]);
        c.decimal = true;
        c.a = 0x99;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x00);
        assert!(c.carry, "decimal carry out of the high nibble");
    }

    #[test]
    fn adc_decimal_flags_do_not_describe_the_decimal_result() {
        // THE NMOS TRAP. 0x99 + 0x01 in decimal gives A = 0x00, but Z is
        // computed from the BINARY sum (0x99 + 0x01 = 0x9A), so Z is CLEAR
        // even though the accumulator is zero. The 65C02 fixed this; the 6502
        // in a 1983 coin-op did not, and CCN.MAC's coin counting runs with D
        // set — so "correcting" it would change behaviour on real code.
        let (mut c, mut b) = run(&[0x69, 0x01]);
        c.decimal = true;
        c.a = 0x99;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x00);
        assert!(!c.zero, "Z comes from the binary sum, not the decimal result");
    }

    #[test]
    fn sbc_decimal_takes_every_flag_from_the_binary_operation() {
        // Asymmetric with ADC, and that asymmetry is real hardware: on NMOS,
        // SBC in decimal mode computes all four flags binary and only the
        // accumulator decimally.
        let (mut c, mut b) = run(&[0xE9, 0x01]);
        c.decimal = true;
        c.carry = true;
        c.a = 0x00;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x99, "0 - 1 in BCD wraps to 99");
        assert!(!c.carry, "borrow, from the binary operation");

        // 46 - 12 = 34.
        let (mut c, mut b) = run(&[0xE9, 0x12]);
        c.decimal = true;
        c.carry = true;
        c.a = 0x46;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x34);
        assert!(c.carry);
    }

    #[test]
    fn compares_set_carry_as_unsigned_ge_and_never_touch_overflow() {
        for (a, m, carry, zero) in [
            (0x50u8, 0x30u8, true, false),
            (0x30, 0x50, false, false),
            (0x40, 0x40, true, true),
            (0xFF, 0x01, true, false),
        ] {
            let (mut c, mut b) = run(&[0xC9, m]);
            c.a = a;
            c.overflow = true;
            c.step(&mut b).unwrap();
            assert_eq!(c.carry, carry, "CMP {a:#04x},{m:#04x} carry");
            assert_eq!(c.zero, zero, "CMP {a:#04x},{m:#04x} zero");
            assert!(c.overflow, "CMP must not touch V");
        }
        // CPX and CPY use their own registers.
        let (mut c, mut b) = run(&[0xE0, 0x05]);
        c.x = 0x05;
        c.step(&mut b).unwrap();
        assert!(c.zero && c.carry);
    }

    #[test]
    fn shifts_and_rotates_move_bits_through_carry() {
        let (mut c, mut b) = run(&[0x0A]);
        c.a = 0x81;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x02);
        assert!(c.carry, "bit 7 shifted out");

        let (mut c, mut b) = run(&[0x4A]);
        c.a = 0x81;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x40);
        assert!(c.carry, "bit 0 shifted out");
        assert!(!c.negative, "LSR always clears N");

        // ROL brings carry in at the bottom and sends bit 7 out.
        let (mut c, mut b) = run(&[0x2A]);
        c.a = 0x80;
        c.carry = true;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x01);
        assert!(c.carry);

        // ROR brings carry in at the top.
        let (mut c, mut b) = run(&[0x6A]);
        c.a = 0x01;
        c.carry = true;
        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x80);
        assert!(c.carry);
        assert!(c.negative);
    }

    #[test]
    fn read_modify_write_hits_memory() {
        // ASL 1+SC.NM / ROL 2+SC.NM — CMN.MAC:295's score doubling. The
        // target must sit outside the program bytes, or the shift rewrites its
        // own operand.
        let (mut c, mut b) = run(&[0x0E, 0x01, 0x03, 0x2E, 0x02, 0x03]);
        b.mem[0x0301] = 0x80;
        b.mem[0x0302] = 0x00;
        step_n(&mut c, &mut b, 2);
        assert_eq!(b.mem[0x0301], 0x00);
        assert_eq!(b.mem[0x0302], 0x01, "carry rotated into the high byte");

        let (mut c, mut b) = run(&[0xE6, 0x10]);
        b.mem[0x0010] = 0xFF;
        c.step(&mut b).unwrap();
        assert_eq!(b.mem[0x0010], 0x00);
        assert!(c.zero, "INC wraps and sets Z");
    }

    #[test]
    fn branches_take_and_cost_correctly() {
        // Not taken: 2 cycles, PC just past the displacement.
        let (mut c, mut b) = run(&[0xD0, 0x10]);
        c.zero = true;
        let n = c.step(&mut b).unwrap();
        assert_eq!(n, 2);
        assert_eq!(c.pc, 0x0202);

        // Taken, same page: 3 cycles.
        let (mut c, mut b) = run(&[0xD0, 0x10]);
        c.zero = false;
        let n = c.step(&mut b).unwrap();
        assert_eq!(n, 3);
        assert_eq!(c.pc, 0x0212);

        // Taken across a page: 4 cycles.
        let mut bus = FlatBus::new();
        bus.load(0x02F0, &[0xD0, 0x40]);
        let mut c = Cpu::new();
        c.pc = 0x02F0;
        c.zero = false;
        let n = c.step(&mut bus).unwrap();
        assert_eq!(n, 4, "page crossing costs an extra cycle");
        assert_eq!(c.pc, 0x0332);
    }

    #[test]
    fn every_branch_tests_its_own_flag() {
        let cases: &[(u8, &str, bool)] = &[
            (0x90, "carry", false),
            (0xB0, "carry", true),
            (0xD0, "zero", false),
            (0xF0, "zero", true),
            (0x10, "negative", false),
            (0x30, "negative", true),
            (0x50, "overflow", false),
            (0x70, "overflow", true),
        ];
        for (op, flagname, taken_when) in cases {
            for state in [false, true] {
                let (mut c, mut b) = run(&[*op, 0x10]);
                match *flagname {
                    "carry" => c.carry = state,
                    "zero" => c.zero = state,
                    "negative" => c.negative = state,
                    _ => c.overflow = state,
                }
                c.step(&mut b).unwrap();
                let took = c.pc == 0x0212;
                assert_eq!(
                    took,
                    state == *taken_when,
                    "opcode {op:#04x} with {flagname}={state}"
                );
            }
        }
    }

    #[test]
    fn jsr_and_rts_round_trip() {
        // JSR $0300 at $0200, then RTS: control must return to $0203.
        let mut b = FlatBus::new();
        b.load(0x0200, &[0x20, 0x00, 0x03, 0xEA]);
        b.load(0x0300, &[0x60]);
        let mut c = Cpu::new();
        c.pc = 0x0200;

        c.step(&mut b).unwrap();
        assert_eq!(c.pc, 0x0300);
        // JSR pushes the address of its LAST byte, $0202 — not the return
        // address. RTS adds one back.
        assert_eq!(b.mem[0x01FD], 0x02, "high byte of $0202");
        assert_eq!(b.mem[0x01FC], 0x02, "low byte of $0202");

        c.step(&mut b).unwrap();
        assert_eq!(c.pc, 0x0203, "returned past the JSR");
        assert_eq!(c.s, 0xFD, "stack balanced");
    }

    #[test]
    fn jmp_indirect_honours_the_page_bug_end_to_end() {
        let mut b = FlatBus::new();
        b.load(0x0200, &[0x6C, 0xFF, 0x30]);
        b.mem[0x30FF] = 0xCD;
        b.mem[0x3000] = 0xAB;
        b.mem[0x3100] = 0x99;
        let mut c = Cpu::new();
        c.pc = 0x0200;
        c.step(&mut b).unwrap();
        assert_eq!(c.pc, 0xABCD);
    }

    #[test]
    fn a_real_loop_from_the_game_runs_to_completion() {
        // CEN.MAC's EN.INI shape: LDX #0 / loop { INX INX } / CPX #8 / BMI loop
        // — HLL65F's BEGIN..PLEND compiles to exactly this.
        let mut b = FlatBus::new();
        b.load(0x0200, &[0xA2, 0x00, 0xE8, 0xE8, 0xE0, 0x08, 0x30, 0xFA]);
        let mut c = Cpu::new();
        c.pc = 0x0200;
        c.step(&mut b).unwrap();
        for _ in 0..64 {
            if c.pc == 0x0208 {
                break;
            }
            c.step(&mut b).unwrap();
        }
        assert_eq!(c.pc, 0x0208, "loop terminated");
        assert_eq!(c.x, 0x08);
    }

    #[test]
    fn indexed_reads_pay_a_cycle_on_a_page_crossing() {
        // LDA $80FF,X with X=1 crosses into $8100: 4 cycles becomes 5.
        let (mut c, mut b) = run(&[0xBD, 0xFF, 0x80]);
        c.x = 0x01;
        assert_eq!(c.step(&mut b).unwrap(), 5);

        // No crossing: the base 4.
        let (mut c, mut b) = run(&[0xBD, 0x00, 0x80]);
        c.x = 0x01;
        assert_eq!(c.step(&mut b).unwrap(), 4);

        // Same for absolute,Y and (zp),Y across the reading instructions.
        let (mut c, mut b) = run(&[0xB9, 0xFF, 0x80]);
        c.y = 0x01;
        assert_eq!(c.step(&mut b).unwrap(), 5);

        let (mut c, mut b) = run(&[0xB1, 0x20]);
        c.y = 0x02;
        b.mem[0x0020] = 0xFF;
        b.mem[0x0021] = 0x80;
        assert_eq!(c.step(&mut b).unwrap(), 6, "base 5 plus the crossing");

        for op in [0x3Du8, 0x1D, 0x5D, 0x7D, 0xFD, 0xDD] {
            let (mut c, mut b) = run(&[op, 0xFF, 0x80]);
            c.x = 0x01;
            assert_eq!(c.step(&mut b).unwrap(), 5, "opcode {op:#04x} crossing");
        }
    }

    #[test]
    fn indexed_writes_never_pay_the_penalty() {
        // STA abs,X is a flat 5 whether it crosses or not — the hardware always
        // spends the extra cycle, so there is nothing to add.
        let (mut c, mut b) = run(&[0x9D, 0xFF, 0x80]);
        c.x = 0x01;
        assert_eq!(c.step(&mut b).unwrap(), 5);
        let (mut c, mut b) = run(&[0x9D, 0x00, 0x80]);
        c.x = 0x01;
        assert_eq!(c.step(&mut b).unwrap(), 5);
    }

    #[test]
    fn documented_cycle_counts() {
        for (prog, want) in [
            (vec![0xA9u8, 0x00], 2u8),  // LDA #
            (vec![0xA5, 0x10], 3),      // LDA zp
            (vec![0xB5, 0x10], 4),      // LDA zp,X
            (vec![0xAD, 0x00, 0x80], 4),// LDA abs
            (vec![0xA1, 0x10], 6),      // LDA (zp,X)
            (vec![0x85, 0x10], 3),      // STA zp
            (vec![0x8D, 0x00, 0x80], 4),// STA abs
            (vec![0x91, 0x10], 6),      // STA (zp),Y
            (vec![0xAA], 2),            // TAX
            (vec![0x48], 3),            // PHA
            (vec![0x68], 4),            // PLA
            (vec![0x08], 3),            // PHP
            (vec![0x28], 4),            // PLP
            (vec![0xE6, 0x10], 5),      // INC zp
            (vec![0xEE, 0x00, 0x80], 6),// INC abs
            (vec![0xFE, 0x00, 0x80], 7),// INC abs,X
            (vec![0x0A], 2),            // ASL A
            (vec![0x06, 0x10], 5),      // ASL zp
            (vec![0x4C, 0x00, 0x80], 3),// JMP abs
            (vec![0x6C, 0x00, 0x80], 5),// JMP (abs)
            (vec![0x20, 0x00, 0x80], 6),// JSR
            (vec![0x60], 6),            // RTS
            (vec![0x40], 6),            // RTI
            (vec![0x00], 7),            // BRK
            (vec![0xEA], 2),            // NOP
        ] {
            let (mut c, mut b) = run(&prog);
            let got = c.step(&mut b).unwrap();
            assert_eq!(got, want, "cycles for {:02x?}", prog);
        }
    }

    #[test]
    fn irq_is_blocked_while_i_is_set_and_delivered_once_cleared() {
        let (mut c, mut b) = run(&[0xEA]);
        b.mem[0xFFFE] = 0x09;
        b.mem[0xFFFF] = 0xE0;

        c.interrupt_disable = true;
        assert_eq!(c.irq(&mut b), 0, "masked");
        assert_eq!(c.pc, 0x0200, "PC untouched");

        c.interrupt_disable = false;
        assert_eq!(c.irq(&mut b), 7);
        assert_eq!(c.pc, 0xE009, "the board's real IRQ target");
        assert!(c.interrupt_disable, "entry masks further IRQs");
    }

    #[test]
    fn nmi_ignores_the_mask_even_though_this_board_never_raises_it() {
        // MicroProcessor.v:26 ties NMI low, so the game can never see one.
        // Implemented for correctness; the scheduler simply never calls it.
        let (mut c, mut b) = run(&[0xEA]);
        b.mem[0xFFFA] = 0x09;
        b.mem[0xFFFB] = 0xE0;
        c.interrupt_disable = true;
        assert_eq!(c.nmi(&mut b), 7);
        assert_eq!(c.pc, 0xE009);
    }

    #[test]
    fn interrupt_entry_does_not_clear_decimal_mode() {
        // The NMOS 6502 leaves D alone on interrupt entry — which is exactly
        // why CIN.MAC:20 opens the game's ISR with `CLD ; set to binary mode`.
        // Clearing it here would make that instruction redundant and hide a
        // real difference from the 65C02.
        let (mut c, mut b) = run(&[0xEA]);
        b.mem[0xFFFE] = 0x00;
        b.mem[0xFFFF] = 0xE0;
        c.decimal = true;
        c.interrupt_disable = false;
        c.irq(&mut b);
        assert!(c.decimal, "D survives interrupt entry on NMOS");
    }

    #[test]
    fn pushed_break_flag_distinguishes_brk_from_irq() {
        // The only way a handler can tell them apart, since B is not stored.
        let (mut c, mut b) = run(&[0x00, 0xEA]);
        b.mem[0xFFFE] = 0x00;
        b.mem[0xFFFF] = 0xE0;
        c.step(&mut b).unwrap();
        assert_eq!(b.mem[0x01FB] & flag::BREAK, flag::BREAK, "BRK sets B");
        // BRK is two bytes: the pushed PC skips the padding byte.
        assert_eq!(b.mem[0x01FC], 0x02, "low byte of $0202");

        let (mut c, mut b) = run(&[0xEA]);
        b.mem[0xFFFE] = 0x00;
        b.mem[0xFFFF] = 0xE0;
        c.interrupt_disable = false;
        c.irq(&mut b);
        assert_eq!(b.mem[0x01FB] & flag::BREAK, 0, "IRQ clears B");
    }

    #[test]
    fn rti_restores_flags_and_resumes_exactly() {
        // Unlike RTS, RTI does not add one to the pulled address.
        let mut b = FlatBus::new();
        b.load(0x0200, &[0x40]);
        b.mem[0x01FD] = 0x02; // PC high
        b.mem[0x01FC] = 0x34; // PC low
        b.mem[0x01FB] = flag::CARRY | flag::NEGATIVE;
        let mut c = Cpu::new();
        c.pc = 0x0200;
        c.s = 0xFA;
        c.step(&mut b).unwrap();
        assert_eq!(c.pc, 0x0234, "resumes exactly, no +1");
        assert!(c.carry && c.negative);
        assert_eq!(c.s, 0xFD, "three bytes pulled");
    }

    #[test]
    fn a_full_interrupt_round_trip_returns_to_the_interrupted_instruction() {
        // Handler at $0300 does nothing but RTI. Execution must resume mid-flow
        // with flags intact — the property the whole creep line depends on.
        let mut b = FlatBus::new();
        b.load(0x0200, &[0xA9, 0x42, 0xA9, 0x99]);
        b.load(0x0300, &[0x40]);
        b.mem[0xFFFE] = 0x00;
        b.mem[0xFFFF] = 0x03;
        let mut c = Cpu::new();
        c.pc = 0x0200;
        c.interrupt_disable = false;

        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x42);
        let resume = c.pc;

        c.irq(&mut b);
        assert_eq!(c.pc, 0x0300);
        c.step(&mut b).unwrap(); // RTI
        assert_eq!(c.pc, resume, "resumed where it left off");
        assert!(!c.interrupt_disable, "I restored from the pushed P");

        c.step(&mut b).unwrap();
        assert_eq!(c.a, 0x99);
        assert_eq!(c.s, 0xFD, "stack balanced across the interrupt");
    }
}
