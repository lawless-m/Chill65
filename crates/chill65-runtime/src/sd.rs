//! The Space Duel bus — a second machine beside the Crystal Castles one.
//!
//! [`crate::machine::Machine`] is not modified or generalised to fit this.
//! The two boards share a CPU and a sound chip and almost nothing else: one
//! draws into a bitmap, the other drives a vector generator; one banks ROM
//! through an addressable latch, the other has a flat map. A shared struct
//! bent to cover both would make each harder to check against its own
//! evidence, and the `Bus` trait is already the seam that lets them differ.
//!
//! # Where the map comes from
//!
//! `space-duel/AS2DEC.MAC`, the game's own hardware declarations, cited per
//! region below. **MAME's `spacduel` ROM layout independently corroborates
//! the ROM half of it** — `136006-106.r7` loads at `2800`, `136006-107.np7`
//! at `3000`, and the program devices from `4000` — which is exactly how the
//! Phase 1 link lays the image out.
//!
//! ```text
//! 0000-07FF  RAM              0000-00FF base page, 0100-01FF stack
//! 0800       status in        3 kHz, VG halt, diag, self-test, slam, coins
//! 0900-0907  switches in      two per address on D7/D6
//! 0A00       EAROM data in
//! 0C00       output latch     coin counters, lockout, lamps, X/Y invert
//! 0C80       GOADD            start the vector generator
//! 0D00       WTCHDG           pet the watchdog
//! 0D80       STOPADD          stop the vector generator
//! 0E00       INTACK           acknowledge the interrupt
//! 0E80       EAROM control
//! 0F00       EAROM address/data latch
//! 1000-100F  POKEY 1
//! 1400-140F  POKEY 2
//! 2000-27FF  vector RAM
//! 2800-3FFF  vector ROM
//! 4000-8FFF  program ROM
//! ```
//!
//! **UNVERIFIED:** how much of the space between the decoded windows mirrors.
//! There is no RTL for this board to transcribe, so only the windows the
//! equates name are decoded, and everything else reads back `0xFF`.
//!
//! # RAM
//!
//! `0000-07FF`, which is everything below the first I/O address. Measured
//! from the link: the highest symbol in low memory is `DIFSW` at `03F9` and
//! nothing at all lives in `0400-07FF`, so the choice cannot matter to the
//! game — **UNVERIFIED** whether the board decodes 1K mirrored or 2K flat.
//!
//! `ASTRD2.MAC`'s `PWRON` does `LDX I,0FE / TXS`, so page 1 is the stack. The
//! game also keeps 18 variables in the low part of that page, which is normal
//! for a 6502 title with a shallow stack.
//!
//! # The status port at 0800
//!
//! Every bit idles **high** except the two the hardware drives:
//!
//! ```text
//! D7  3 kHz signal     AS2DEC.MAC:6    toggles; see THREE_KHZ_HALF_PERIOD
//! D6  VG halt          AS2DEC.MAC:7    1 = halted
//! D5  diagnostic step  AS2DEC.MAC:8    0 = pushed
//! D4  self test        AS2DEC.MAC:9    0 = on
//! D3  slam             AS2DEC.MAC:10   0 = slammed
//! D2  left coin        AS2DEC.MAC:11   0 = coin present
//! D1  centre coin      AS2DEC.MAC:12
//! D0  right coin       AS2DEC.MAC:13
//! ```
//!
//! The polarities are read off the game rather than assumed. `AS2TST.MAC:835`
//! reads the port, does `EOR #0FF`, then masks slam and diag and asks "SWITCH
//! PUSHED?" — so a *zero* in the raw byte is a press. `COIN65.MAC:361-364`
//! agrees for slam from the other direction: `AS2COI` sets `SLAM=0`, which
//! selects `BNE 2$ ;BRANCH IF BIT HI (SWITCH OFF)`. And `COIN65.MAC:50` says
//! `COIN=0 ;COIN IS LOW-TRUE`, which selects `BCS 5$ ;BRANCH IF INPUT HIGH
//! (COIN ABSENT)` at `COIN65.MAC:339`.
//!
//! D6 is the one the main loop waits on: `ASTRD2.MAC:723` is `BIT HALT / BVC`,
//! and `BIT` copies bit 6 into V, so it spins until the bit **sets**. Halted
//! is 1.
//!
//! # The vector generator
//!
//! `ASTRD2.MAC:7508-7514` (`INITVG`) builds the entry point:
//!
//! ```text
//! VECRAM   = 01      word 0 low byte
//! VECRAM+1 = 0E0     word 0 high byte  ->  JMPL to word 001 (CPU 2002)
//! VECRAM+3 = 020     word 1 = HALT
//! VECRAM+403 = 020   word 201 = HALT
//! ```
//!
//! So **execution starts at word 0**, which holds a jump. The main loop's
//! buffer swap is `LDA VECRAM+1 / EOR I,2 / STA VECRAM+1 / STA GOADD`
//! (`ASTRD2.MAC:744-747`): toggling bit 1 of that high byte moves the jump
//! target between word `001` and word `201` — CPU `2002` and `2402`, the two
//! build buffers `ASTRD2.MAC:748-752` picks between.
//!
//! `GOADD` here runs the list to completion at once. That is a documented
//! simplification: the CPU-visible contract is the halt bit alone, and the
//! game only ever strobes `GO` after seeing it set, so no code in the corpus
//! can observe a partially drawn frame. **UNVERIFIED:** how long a real list
//! takes to draw.

use crate::bus::Bus;
use crate::pokey::Pokey;
use crate::vg::{Segment, Stop, Vg, VEC_BYTES};

/// CPU clock, in Hz.
///
/// Measured: `mame -listxml spacduel` reports the `maincpu` M6502 at
/// `clock="1512000"`.
pub const CPU_HZ: u32 = 1_512_000;

/// Half the period of the 3 kHz signal on `0800` D7, in CPU cycles.
///
/// `AS2DEC.MAC:6` names the signal `THRKHZ ;(D7) 3KHZ SIGNAL`, and the clock
/// divides exactly: 1,512,000 / 3,000 = 504 cycles per period, 252 per half.
/// That the division is exact is itself evidence the reading is right.
///
/// **UNVERIFIED:** the divider chain that produces it. Only the frequency is
/// stated by the source.
pub const THREE_KHZ_HALF_PERIOD: u64 = (CPU_HZ / 3_000 / 2) as u64;

/// How many cycles the watchdog tolerates between strobes.
///
/// Same reasoning as [`crate::machine::DEFAULT_WATCHDOG_CYCLES`]: the board's
/// true period is not established, so this is chosen from the game's own
/// cadence. `ASTRD2.MAC:728` strobes once per `SYNC` — once per frame — which
/// is on the order of 24,000 cycles. 200,000 leaves an eightfold margin.
pub const DEFAULT_WATCHDOG_CYCLES: u32 = 200_000;

/// Instruction budget for one `GOADD` strobe.
///
/// Generous: the longest real list measured is well under a thousand. This
/// only exists so a malformed display list reports rather than hangs.
pub const VG_BUDGET: u64 = 200_000;

const RAM_LEN: usize = 0x0800;
const VEC_RAM_LEN: usize = 0x0800;
const VEC_ROM_LEN: usize = 0x1800;
/// `4000-8FFF`.
pub const PROG_LEN: usize = 0x5000;
/// The ship image, CPU `2800-2FFF`.
pub const SHIP_LEN: usize = 0x0800;
/// The Phase 1 program link, CPU `0000-8FFF`.
pub const IMAGE_LEN: usize = 0x9000;

/// Player switches and option lines, read through `0900-0907`.
///
/// Two per address on D7 and D6, and unlike the status port these are
/// **active high**: `AS2DEC.MAC:15-32` says "D7=1 FOR ON" throughout.
#[derive(Clone, Copy, Debug, Default)]
pub struct SdInput {
    /// `0900` D7 — shields. `AS2DEC.MAC:15` still calls it `HYPSW`.
    pub shield: bool,
    /// `0900` D6.
    pub fire: bool,
    /// `0902` D7 / D6.
    pub rotate_left: bool,
    pub rotate_right: bool,
    /// `0904` D7 / D6.
    pub thrust: bool,
    pub start: bool,
    /// `0906` D7.
    pub game_select: bool,
    /// `0907` D7 — 1 upright, 0 cocktail (`AS2DEC.MAC:31`).
    pub upright: bool,
    /// Option lines on D6 of `0905`, `0906`, `0907`: sell-games, two-coin
    /// minimum, cabaret (`AS2DEC.MAC:26,29,32`).
    pub sell_games: bool,
    pub two_coin_minimum: bool,
    pub cabaret: bool,
}

impl SdInput {
    /// An upright cabinet with nothing pressed.
    pub fn upright() -> Self {
        SdInput {
            upright: true,
            ..Default::default()
        }
    }

    fn read(&self, offset: u16) -> u8 {
        let (d7, d6) = match offset & 7 {
            0 => (self.shield, self.fire),
            2 => (self.rotate_left, self.rotate_right),
            4 => (self.thrust, self.start),
            5 => (false, self.sell_games),
            6 => (self.game_select, self.two_coin_minimum),
            7 => (self.upright, self.cabaret),
            // 0901 and 0903 are the second player's switches on a cocktail
            // cabinet; `AS2DEC.MAC:17-22` has them commented out, so this
            // build never reads them.
            _ => (false, false),
        };
        (d7 as u8) << 7 | (d6 as u8) << 6
    }
}

/// The EAROM, as a seam rather than a model.
///
/// `AS2DEC.MAC:71-84` gives the address, control and latch ports and the
/// C1/C2 truth table. Persistence is the embedder's business — the same
/// decision `harness.md` records for Crystal Castles — so writes are accepted
/// and remembered for the length of a run and nothing reaches a filesystem.
#[derive(Clone, Debug)]
pub struct Earom {
    pub control: u8,
    pub latch: u8,
    pub cells: [u8; 0x40],
}

impl Default for Earom {
    fn default() -> Self {
        Earom {
            control: 0,
            latch: 0,
            cells: [0; 0x40],
        }
    }
}

pub struct SdMachine {
    pub ram: [u8; RAM_LEN],
    pub vec_ram: [u8; VEC_RAM_LEN],
    pub vec_rom: [u8; VEC_ROM_LEN],
    pub prog: [u8; PROG_LEN],

    pub vg: Vg,
    /// The segments the last `GOADD` strobe drew.
    pub segments: Vec<Segment>,
    pub vg_go_strobes: u64,
    /// Set if a display list ever failed to end cleanly. Reported, never a
    /// panic: a list is data, and malformed data must not stop the machine.
    pub vg_fault: Option<Stop>,

    pub pokey0: Pokey,
    pub pokey1: Pokey,
    pub input: SdInput,
    pub earom: Earom,

    /// The `0C00` byte latch: coin counters D0-D2, lockout D3, select and
    /// start lamps D4-D5 (0 = on), X invert D6, Y invert D7.
    ///
    /// One latch, not eight one-bit addresses — `AS2DEC.MAC:38-49` equates
    /// `$COINCOUNTER`, `$LOCKOUT`, `INVERTS` and `OUT1` all to `0C00`, which
    /// is what tells them apart from Crystal Castles' addressable latches.
    pub out_latch: u8,

    /// Diagnostic step switch, `0800` D5. Idle is *not pushed*.
    pub diag_pushed: bool,
    /// Self-test switch, `0800` D4. `AS2DEC.MAC:9`: 0 = on.
    pub self_test: bool,
    /// Slam switch, `0800` D3.
    pub slammed: bool,
    /// Coin senses, `0800` D2-D0, one bit each, low-true on the bus.
    pub coins: u8,

    pub watchdog: u32,
    pub watchdog_limit: u32,
    pub watchdog_expired: bool,
    pub watchdog_strobes: u64,
    pub irq_pending: bool,

    pub cycles: u64,
}

impl Default for SdMachine {
    fn default() -> Self {
        SdMachine::new()
    }
}

impl SdMachine {
    pub fn new() -> Self {
        SdMachine {
            ram: [0; RAM_LEN],
            vec_ram: [0; VEC_RAM_LEN],
            vec_rom: [0; VEC_ROM_LEN],
            prog: [0; PROG_LEN],
            vg: Vg::new(),
            segments: Vec::new(),
            vg_go_strobes: 0,
            vg_fault: None,
            pokey0: Pokey::new(),
            pokey1: Pokey::new(),
            input: SdInput::upright(),
            earom: Earom::default(),
            out_latch: 0,
            diag_pushed: false,
            self_test: false,
            slammed: false,
            coins: 0,
            watchdog: 0,
            watchdog_limit: DEFAULT_WATCHDOG_CYCLES,
            watchdog_expired: false,
            watchdog_strobes: 0,
            irq_pending: false,
            cycles: 0,
        }
    }

    /// Load the images the Phase 1 assembler produces.
    ///
    /// `ship` is `A2SHIP.LDA`'s span, CPU `2800-2FFF`. `image` is the
    /// fifteen-module link, CPU `0000-8FFF`, from which `3000-3FFF` becomes
    /// the rest of vector ROM and `4000-8FFF` the program.
    pub fn load_roms(&mut self, ship: &[u8], image: &[u8]) -> Result<(), String> {
        if ship.len() != SHIP_LEN {
            return Err(format!(
                "ship image is {} bytes, expected {SHIP_LEN} (2800-2FFF)",
                ship.len()
            ));
        }
        if image.len() != IMAGE_LEN {
            return Err(format!(
                "program image is {} bytes, expected {IMAGE_LEN} (0000-8FFF)",
                image.len()
            ));
        }
        self.vec_rom[..SHIP_LEN].copy_from_slice(ship);
        self.vec_rom[SHIP_LEN..].copy_from_slice(&image[0x3000..0x4000]);
        self.prog.copy_from_slice(&image[0x4000..0x9000]);
        Ok(())
    }

    /// Set the two option bytes the game reads through `ALLPOT`.
    ///
    /// `AS2DEC.MAC:149,159`: OPTN1 on POKEY 1, OPTN2 on POKEY 2.
    pub fn set_options(&mut self, optn1: u8, optn2: u8) {
        self.pokey0.pot_lines = optn1;
        self.pokey1.pot_lines = optn2;
    }

    /// The generator's 8K window, CPU `2000-3FFF`, as one slice.
    fn vector_window(&self) -> Vec<u8> {
        let mut mem = Vec::with_capacity(VEC_BYTES);
        mem.extend_from_slice(&self.vec_ram);
        mem.extend_from_slice(&self.vec_rom);
        mem
    }

    /// Run the display list from word 0. See the module note on `INITVG`.
    fn vg_go(&mut self) {
        let mem = self.vector_window();
        self.vg.reset(0);
        self.segments.clear();
        let stop = self.vg.run(&mem, VG_BUDGET, &mut self.segments);
        if !stop.is_clean() && self.vg_fault.is_none() {
            self.vg_fault = Some(stop);
        }
        // Whatever happened, the CPU sees a finished frame. A list that
        // faulted has still stopped running.
        self.vg.halted = true;
        self.vg_go_strobes += 1;
    }

    /// Strobe the watchdog, as a write to `0D00` does.
    pub fn pet_watchdog(&mut self) {
        self.watchdog = 0;
        self.watchdog_strobes += 1;
    }

    /// The status byte at `0800`. See the module note for every bit.
    fn status(&self) -> u8 {
        let mut v = 0u8;
        if (self.cycles / THREE_KHZ_HALF_PERIOD) & 1 == 1 {
            v |= 0x80;
        }
        if self.vg.halted {
            v |= 0x40;
        }
        if !self.diag_pushed {
            v |= 0x20;
        }
        if !self.self_test {
            v |= 0x10;
        }
        if !self.slammed {
            v |= 0x08;
        }
        // Low-true: a set bit is no coin.
        v |= (!self.coins) & 0x07;
        v
    }

    /// A stable identity for the current picture.
    ///
    /// FNV-1a over the segment list, serialised canonically. Reuses
    /// [`crate::video::fnv1a`] so both machines hash the same way; nothing
    /// inside `video.rs` changes.
    pub fn frame_hash(&self) -> u64 {
        let mut bytes = Vec::with_capacity(self.segments.len() * 20);
        for s in &self.segments {
            for v in [s.x0, s.y0, s.x1, s.y1] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            bytes.push(s.intensity);
            bytes.push(s.color);
        }
        crate::video::fnv1a(&bytes)
    }
}

impl Bus for SdMachine {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x07FF => self.ram[addr as usize],
            0x0800..=0x08FF => self.status(),
            0x0900..=0x09FF => self.input.read(addr),
            0x0A00..=0x0AFF => self.earom.cells[(self.earom.latch & 0x3F) as usize],
            0x1000..=0x13FF => self.pokey0.read((addr & 0x0F) as usize, self.cycles),
            0x1400..=0x17FF => self.pokey1.read((addr & 0x0F) as usize, self.cycles),
            0x2000..=0x27FF => self.vec_ram[(addr - 0x2000) as usize],
            0x2800..=0x3FFF => self.vec_rom[(addr - 0x2800) as usize],
            0x4000..=0x8FFF => self.prog[(addr - 0x4000) as usize],
            // The 6502's vectors live at 8FFA-8FFF in the linked image, so
            // the top page must answer for them. Only the vector fetch is
            // load-bearing — all code and data sit below 0x9000.
            //
            // UNVERIFIED: the real decode. This mirrors the last ROM page.
            0x9000..=0xFFFF => self.prog[PROG_LEN - 0x1000 + (addr as usize & 0x0FFF)],
            _ => 0xFF,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x07FF => self.ram[addr as usize] = value,
            0x0C00..=0x0C7F => self.out_latch = value,
            0x0C80..=0x0CFF => self.vg_go(),
            0x0D00..=0x0D7F => self.pet_watchdog(),
            0x0D80..=0x0DFF => self.vg.halted = true,
            0x0E00..=0x0E7F => self.irq_pending = false,
            0x0E80..=0x0EFF => self.earom.control = value,
            0x0F00..=0x0FFF => self.earom.latch = value,
            0x1000..=0x13FF => self.pokey0.write((addr & 0x0F) as usize, value, self.cycles),
            0x1400..=0x17FF => self.pokey1.write((addr & 0x0F) as usize, value, self.cycles),
            0x2000..=0x27FF => self.vec_ram[(addr - 0x2000) as usize] = value,
            // ROM. Writes are dropped, as the hardware drops them.
            _ => {}
        }
    }

    fn tick(&mut self, cycles: u8) {
        self.cycles += cycles as u64;
        self.watchdog = self.watchdog.saturating_add(cycles as u32);
        if self.watchdog >= self.watchdog_limit {
            self.watchdog_expired = true;
        }
    }

    /// Read without side effects.
    ///
    /// The POKEY registers and the status port are the reason this exists:
    /// reading `RANDOM` advances nothing here, but a debugger poking at
    /// `0C80` through `read` would start the vector generator.
    fn peek(&mut self, addr: u16) -> u8 {
        match addr {
            0x1000..=0x17FF => 0xFF,
            _ => self.read(addr),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> SdMachine {
        let mut m = SdMachine::new();
        m.load_roms(&[0u8; SHIP_LEN], &[0u8; IMAGE_LEN]).unwrap();
        m
    }

    /// Plant little-endian words into vector RAM starting at word `at`.
    fn put_words(m: &mut SdMachine, at: usize, words: &[u16]) {
        for (i, w) in words.iter().enumerate() {
            m.vec_ram[(at + i) * 2] = *w as u8;
            m.vec_ram[(at + i) * 2 + 1] = (*w >> 8) as u8;
        }
    }

    const HALT: u16 = 0x2000;
    const RTSL: u16 = 0xC000;
    const CNTR: u16 = 0x8040;
    fn vctr(dx: i32, dy: i32, zz: u8) -> u16 {
        0x4000 + ((zz as u16) << 5) + ((dx / 2) as u16 & 0x1F) + (((dy as u16) << 7) & 0x1F00)
    }

    #[test]
    fn ram_and_vector_ram_round_trip_but_rom_does_not() {
        let mut m = machine();
        m.write(0x0000, 0x11);
        m.write(0x07FF, 0x22);
        assert_eq!(m.read(0x0000), 0x11);
        assert_eq!(m.read(0x07FF), 0x22);

        m.write(0x2000, 0x33);
        m.write(0x27FF, 0x44);
        assert_eq!(m.read(0x2000), 0x33);
        assert_eq!(m.read(0x27FF), 0x44);

        // Vector ROM and program ROM ignore writes.
        m.write(0x2800, 0x55);
        m.write(0x4000, 0x66);
        assert_eq!(m.read(0x2800), 0);
        assert_eq!(m.read(0x4000), 0);
    }

    #[test]
    fn the_rom_images_land_where_the_map_says() {
        let mut m = SdMachine::new();
        let mut ship = [0u8; SHIP_LEN];
        ship[0] = 0xA1;
        ship[SHIP_LEN - 1] = 0xA2;
        let mut image = [0u8; IMAGE_LEN];
        image[0x3000] = 0xB1; // first vector-ROM byte from the program link
        image[0x3FFF] = 0xB2;
        image[0x4000] = 0xC1; // first program byte
        image[0x8FFF] = 0xC2;
        m.load_roms(&ship, &image).unwrap();

        assert_eq!(m.read(0x2800), 0xA1);
        assert_eq!(m.read(0x2FFF), 0xA2);
        assert_eq!(m.read(0x3000), 0xB1);
        assert_eq!(m.read(0x3FFF), 0xB2);
        assert_eq!(m.read(0x4000), 0xC1);
        assert_eq!(m.read(0x8FFF), 0xC2);
    }

    #[test]
    fn a_wrong_sized_image_is_refused() {
        let mut m = SdMachine::new();
        assert!(m.load_roms(&[0; 10], &[0; IMAGE_LEN]).is_err());
        assert!(m.load_roms(&[0; SHIP_LEN], &[0; 10]).is_err());
    }

    #[test]
    fn the_cpu_vectors_come_from_the_top_of_the_program_rom() {
        // The link puts them at 8FFA-8FFF; the CPU fetches from FFFA-FFFF.
        let mut m = SdMachine::new();
        let mut image = [0u8; IMAGE_LEN];
        image[0x8FFC] = 0x34; // reset vector low
        image[0x8FFD] = 0x52; // reset vector high
        image[0x8FFE] = 0x78; // IRQ vector low
        image[0x8FFF] = 0x56;
        m.load_roms(&[0; SHIP_LEN], &image).unwrap();

        assert_eq!(m.read_u16(0xFFFC), 0x5234, "reset");
        assert_eq!(m.read_u16(0xFFFE), 0x5678, "IRQ");
        // And the same bytes are readable at their real addresses.
        assert_eq!(m.read(0x8FFC), 0x34);
    }

    #[test]
    fn the_status_port_idles_high_except_where_hardware_drives_it() {
        let mut m = machine();
        m.vg.halted = false;
        m.cycles = 0;
        // D7 low in the first half-period, everything else idle high, D6
        // clear because the generator is running.
        assert_eq!(m.read(0x0800), 0x3F);

        m.cycles = THREE_KHZ_HALF_PERIOD;
        assert_eq!(m.read(0x0800) & 0x80, 0x80, "the 3 kHz bit toggled");
        m.cycles = THREE_KHZ_HALF_PERIOD * 2;
        assert_eq!(m.read(0x0800) & 0x80, 0, "and back");

        m.cycles = 0;
        m.vg.halted = true;
        assert_eq!(m.read(0x0800) & 0x40, 0x40, "halted is 1");

        // Each switch pulls its bit low. AS2TST:835 inverts before testing.
        m.diag_pushed = true;
        assert_eq!(m.read(0x0800) & 0x20, 0);
        m.self_test = true;
        assert_eq!(m.read(0x0800) & 0x10, 0);
        m.slammed = true;
        assert_eq!(m.read(0x0800) & 0x08, 0);
        m.coins = 0b010;
        assert_eq!(m.read(0x0800) & 0x07, 0b101, "coins are low-true");
    }

    #[test]
    fn switches_read_active_high_two_per_address() {
        let mut m = machine();
        m.input = SdInput::default();
        for a in 0x0900..=0x0907u16 {
            assert_eq!(m.read(a) & 0xC0, 0, "{a:04X} idle");
        }
        m.input.shield = true;
        assert_eq!(m.read(0x0900) & 0xC0, 0x80);
        m.input.fire = true;
        assert_eq!(m.read(0x0900) & 0xC0, 0xC0);
        m.input.rotate_left = true;
        assert_eq!(m.read(0x0902) & 0xC0, 0x80);
        m.input.thrust = true;
        m.input.start = true;
        assert_eq!(m.read(0x0904) & 0xC0, 0xC0);
        m.input.upright = true;
        assert_eq!(m.read(0x0907) & 0x80, 0x80, "upright is 1");
    }

    #[test]
    fn the_output_latch_is_one_byte_not_eight_addresses() {
        let mut m = machine();
        m.write(0x0C00, 0xC5);
        assert_eq!(m.out_latch, 0xC5);
        assert_eq!(m.out_latch & 0x40, 0x40, "XINVERT");
        assert_eq!(m.out_latch & 0x80, 0x80, "YINVERT");
    }

    #[test]
    fn intack_clears_the_interrupt_and_the_watchdog_can_starve() {
        let mut m = machine();
        m.irq_pending = true;
        m.write(0x0E00, 0);
        assert!(!m.irq_pending);

        assert!(!m.watchdog_expired);
        for _ in 0..(DEFAULT_WATCHDOG_CYCLES / 100 + 1) {
            m.tick(100);
        }
        assert!(m.watchdog_expired, "nothing fed it");

        // Strobing keeps a healthy machine alive.
        let mut m = machine();
        for _ in 0..1000 {
            for _ in 0..100 {
                m.tick(100);
            }
            m.write(0x0D00, 0);
        }
        assert!(!m.watchdog_expired);
        assert_eq!(m.watchdog_strobes, 1000);
    }

    #[test]
    fn options_reach_allpot_through_the_bus() {
        let mut m = machine();
        m.set_options(0x5A, 0xA5);
        // ALLPOT is register 8 on each chip — AS2DEC.MAC:111.
        assert_eq!(m.read(0x1008), 0x5A);
        assert_eq!(m.read(0x1408), 0xA5);
    }

    #[test]
    fn goadd_runs_the_list_from_word_zero_and_halts() {
        let mut m = machine();
        // The shape INITVG builds: word 0 jumps to the active buffer.
        put_words(&mut m, 0x000, &[0xE001]);
        put_words(&mut m, 0x001, &[CNTR, vctr(8, 4, 7), vctr(-4, 0, 7), HALT]);

        assert!(m.vg.halted, "power-on state: the game waits on HALT first");
        assert_eq!(m.read(0x0800) & 0x40, 0x40);

        m.write(0x0C80, 0);
        assert_eq!(m.vg_go_strobes, 1);
        assert_eq!(m.segments.len(), 2);
        assert!(m.vg_fault.is_none());
        assert_eq!(m.read(0x0800) & 0x40, 0x40, "halted again afterwards");

        // The hash tracks the picture.
        let first = m.frame_hash();
        put_words(&mut m, 0x001, &[CNTR, vctr(8, 4, 7), vctr(-6, 0, 7), HALT]);
        m.write(0x0C80, 0);
        assert_ne!(m.frame_hash(), first, "a different list hashes differently");
    }

    #[test]
    fn the_buffer_swap_moves_which_list_runs() {
        // ASTRD2.MAC:744-747 toggles bit 1 of the high byte of word 0, which
        // moves the jump between word 001 (CPU 2002) and word 201 (CPU 2402).
        let mut m = machine();
        put_words(&mut m, 0x000, &[0xE001]);
        put_words(&mut m, 0x001, &[vctr(8, 0, 7), HALT]);
        put_words(&mut m, 0x201, &[vctr(4, 0, 7), vctr(4, 0, 7), HALT]);

        m.write(0x0C80, 0);
        assert_eq!(m.segments.len(), 1, "lower buffer");

        let hi = m.read(0x2001);
        m.write(0x2001, hi ^ 0x02);
        m.write(0x0C80, 0);
        assert_eq!(m.segments.len(), 2, "upper buffer after the swap");
    }

    #[test]
    fn a_list_reaching_into_vector_rom_still_returns() {
        let mut m = SdMachine::new();
        let mut ship = [0u8; SHIP_LEN];
        // A shape in the ship ROM at CPU 2800 = word 0x400.
        ship[0] = vctr(6, 0, 7) as u8;
        ship[1] = (vctr(6, 0, 7) >> 8) as u8;
        ship[2] = RTSL as u8;
        ship[3] = (RTSL >> 8) as u8;
        m.load_roms(&ship, &[0u8; IMAGE_LEN]).unwrap();

        put_words(&mut m, 0x000, &[0xE001]);
        put_words(&mut m, 0x001, &[0xA400, HALT]); // JSRL word 0x400
        m.write(0x0C80, 0);
        assert_eq!(m.segments.len(), 1);
        assert!(m.vg_fault.is_none());
    }

    #[test]
    fn stopadd_halts_and_a_runaway_list_faults_without_panicking() {
        let mut m = machine();
        m.vg.halted = false;
        m.write(0x0D80, 0);
        assert!(m.vg.halted);

        // Word 0 jumps to itself: a list that never ends.
        let mut m = machine();
        put_words(&mut m, 0x000, &[0xE000]);
        m.write(0x0C80, 0);
        assert_eq!(m.vg_fault, Some(Stop::Budget));
        assert!(m.vg.halted, "the CPU still sees a finished frame");
    }

    #[test]
    fn peek_does_not_start_the_generator_or_disturb_the_chips() {
        let mut m = machine();
        put_words(&mut m, 0x000, &[0xE001]);
        put_words(&mut m, 0x001, &[vctr(8, 0, 7), HALT]);

        for a in [0x0C80u16, 0x0D00, 0x0D80, 0x0E00] {
            m.peek(a);
        }
        assert_eq!(m.vg_go_strobes, 0, "peeking must not strobe GO");
        assert_eq!(m.watchdog_strobes, 0);

        m.irq_pending = true;
        m.peek(0x0E00);
        assert!(m.irq_pending, "peeking must not acknowledge an interrupt");
    }
}
