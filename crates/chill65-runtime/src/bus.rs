//! The CPU's view of the machine.
//!
//! Everything outside the processor reaches it through this one trait: RAM,
//! banked ROM, the addressable latches, the POKEYs, the bitmap window. Keeping
//! it a trait rather than a concrete type is what lets the CPU be tested
//! against a flat array while the real machine wires up hardware behind the
//! same three methods.

/// Memory and I/O as the 6502 sees it.
///
/// Reads and writes are **side-effecting by design**. On this board a read can
/// change machine state (the POKEY `RANDOM` register advances; `INTACK`
/// acknowledges an interrupt) and a write to a bare address can flip a latch
/// bit without storing anything — the OUT0 latch at `9E80`–`9E87` takes one bit
/// per *address*, from D0 only. An implementation that treats memory as a
/// passive array will appear to work and then diverge.
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;

    fn write(&mut self, addr: u16, value: u8);

    /// Advance machine time by `cycles`.
    ///
    /// Called by the CPU as it executes. Hardware that runs on the machine
    /// clock — the video counters that raise IRQ, the watchdog, the POKEY
    /// LFSR — advances from here, which is what makes the interpreter
    /// deterministic: nothing is driven by host time.
    fn tick(&mut self, cycles: u8) {
        let _ = cycles;
    }

    /// Announce the instruction about to execute, by its address.
    ///
    /// Default no-op, and [`FlatBus`] leaves it that way. [`crate::Machine`]
    /// uses it to attribute bitmap writes to the instruction that made them,
    /// which is what turns "these pixels differ" into "this routine drew them".
    fn begin_instruction(&mut self, pc: u16) {
        let _ = pc;
    }

    /// Read without side effects, for debuggers and tests.
    ///
    /// Defaults to [`Bus::read`]; implementations with side-effecting reads
    /// should override it so inspection cannot perturb the machine.
    fn peek(&mut self, addr: u16) -> u8 {
        self.read(addr)
    }

    /// Little-endian 16-bit read. The 6502 stores addresses low byte first.
    fn read_u16(&mut self, addr: u16) -> u16 {
        let lo = self.read(addr) as u16;
        let hi = self.read(addr.wrapping_add(1)) as u16;
        lo | (hi << 8)
    }
}

/// A flat 64K address space. For CPU tests, not for the machine.
pub struct FlatBus {
    pub mem: Box<[u8; 0x10000]>,
    pub cycles: u64,
}

impl Default for FlatBus {
    fn default() -> Self {
        Self::new()
    }
}

impl FlatBus {
    pub fn new() -> Self {
        FlatBus {
            mem: Box::new([0u8; 0x10000]),
            cycles: 0,
        }
    }

    /// Place `bytes` at `addr`, for loading test programs.
    pub fn load(&mut self, addr: u16, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            self.mem[(addr as usize + i) & 0xFFFF] = *b;
        }
    }

    /// Set the reset vector at `FFFC`.
    pub fn set_reset_vector(&mut self, addr: u16) {
        self.mem[0xFFFC] = addr as u8;
        self.mem[0xFFFD] = (addr >> 8) as u8;
    }
}

impl Bus for FlatBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }

    fn write(&mut self, addr: u16, value: u8) {
        self.mem[addr as usize] = value;
    }

    fn tick(&mut self, cycles: u8) {
        self.cycles += cycles as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_bus_round_trips() {
        let mut b = FlatBus::new();
        b.write(0x1234, 0xAB);
        assert_eq!(b.read(0x1234), 0xAB);
        assert_eq!(b.peek(0x1234), 0xAB);
    }

    #[test]
    fn reads_16_bit_little_endian() {
        let mut b = FlatBus::new();
        b.load(0x0200, &[0x34, 0x12]);
        assert_eq!(b.read_u16(0x0200), 0x1234);
    }

    #[test]
    fn wraps_at_the_top_of_the_address_space() {
        let mut b = FlatBus::new();
        b.write(0xFFFF, 0x0D);
        b.write(0x0000, 0xF0);
        // The high byte comes from 0000, not from past the end.
        assert_eq!(b.read_u16(0xFFFF), 0xF00D);
    }

    #[test]
    fn ticks_accumulate() {
        let mut b = FlatBus::new();
        b.tick(3);
        b.tick(4);
        assert_eq!(b.cycles, 7);
    }
}
