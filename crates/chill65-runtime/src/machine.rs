//! The Crystal Castles machine bus.
//!
//! Address decode transcribed from `Arcade-CrystalCastles_MiSTer/rtl/
//! AddresDecoders.v` and corroborated against the game's own symbols in
//! `hardware.md` §1–§4.
//!
//! ```text
//! 0000-7FFF  RAM — and the bitmap. Not cleared per frame; only between levels.
//! 8000-8FFF  SRAM        (NRn & ~BA[12])
//! 9000-93FF  NVRAM/EAROM (BA[11:10] = 00), 256 bytes mirrored across 1K
//! 9400-97FF  inputs      (BA[11:10] = 01)
//! 9800-9BFF  POKEY x2    (BA[11:10] = 10, split by BA9)
//! 9C00-9C7F  UART        9C80-9CFF  HSLD
//! 9D00-9D7F  VSLD        9D80-9DFF  INTACK
//! 9E00-9E7F  WDOG        9E80-9EFF  OUT0 latch
//! 9F00-9F7F  OUT1 latch  9F80-9FFF  CRAM
//! A000-DFFF  banked ROM — program or castle data
//! E000-FFFF  fixed ROM, never banked
//! ```
//!
//! # The bank latch, and why reads must honour it
//!
//! `CoinCountOutput.v:32` gives `BANK0n = q[7]`, and `ProgramMemory.v:126`
//! selects `~BANK0n ? ic1L : ic1F` — so **q7 clear selects the program, q7 set
//! selects the castle data**, and reset clears the latch, which is why the
//! board wakes up in program.
//!
//! The latch is **write-only**. The machine cannot ask which bank is live, so
//! `CIN.MAC:35-41`'s interrupt handler reads `A000` and tests it for zero: the
//! castle data begins `00`, the program begins with the ATARI copyright string.
//! A bus that ignores the bank on reads will hand the ISR the wrong answer and
//! it will silently restore the wrong bank — no error, just a game that goes
//! wrong later. That is the single most important thing in this file.

use crate::bus::Bus;
use crate::input::Input;
use crate::pokey::Pokey;
use crate::video::{Video, HEIGHT, ROW_BYTES, WIDTH};

/// How many cycles the watchdog tolerates between strobes.
///
/// The board's true timeout is not established from the schematics, so this is
/// chosen from the game's own behaviour instead — which is the number that
/// actually matters, since the point is to catch a wedged machine without ever
/// troubling a healthy one. The two longest gaps between strobes are:
///
/// - the interrupt handler, `CIN.MAC:83` (`STA HW.WDC ; woof`), once per
///   interrupt — a few thousand cycles;
/// - the self-test's ROM checksum, `CST.MAC:344` — the inner loop covers a page
///   and strobes once per page, so ~256 iterations of `EOR NY / INY`, on the
///   order of 2,500 cycles.
///
/// 200,000 leaves roughly a 35× margin over the worst healthy gap. If a future
/// slice finds the real hardware period, replace this; nothing depends on the
/// exact value, only on it being comfortably above the game's own cadence.
pub const DEFAULT_WATCHDOG_CYCLES: u32 = 200_000;

pub struct Machine {
    /// `0000-7FFF`. Also the bitmap; the coordinate window over `0000-0002`
    /// arrives with the video model.
    pub ram: Box<[u8; 0x8000]>,
    /// `8000-8FFF`.
    pub sram: Box<[u8; 0x1000]>,
    /// `9000-93FF`, 256 bytes mirrored. A RAM-backed stub: the store and recall
    /// strobes are accepted, nothing persists.
    pub earom: Box<[u8; 0x100]>,

    /// `A000-FFFF` in bank 0 — the program image.
    pub prog: Box<[u8; 0x6000]>,
    /// `A000-DFFF` in bank 1 — the castle data image.
    pub data: Box<[u8; 0x4000]>,

    /// The motion-object picture ROMs, `136022-106.8d` then `136022-107.8b`,
    /// 8192 bytes each.
    ///
    /// **Not on the CPU bus.** These are read by the video hardware alone, so
    /// unlike `prog` and `data` they never appear in the memory map — which is
    /// why they arrive through their own loader rather than through
    /// [`Machine::load_roms`].
    pub mob_rom: Box<[u8; 0x4000]>,
    /// Whether [`Machine::load_motion_roms`] has been called.
    ///
    /// False means motion objects render as absent everywhere, so a machine
    /// built without them — every unit test, and the committed fixture program
    /// — behaves exactly as it did before they existed.
    pub motion_roms_loaded: bool,

    /// The OUT0 addressable latch. One bit per address, taken from D0.
    ///
    /// Known bits, from the game's equates in `CG.MAC`: bit 0 `HW.TL` = `9E80`
    /// trackball light, bit 7 `HW.BSL` = `9E87` bank select — commented there
    /// as "bank select 0,FF", which is exactly the D0-latched write.
    pub out0: u8,
    /// The OUT1 addressable latch: `9F00` `HW.AX`, `9F01` `HW.AY`, `9F02`
    /// `HW.XIN`, `9F03` `HW.YIN`, `9F04` `HW.FLP` (cocktail flip, `PLAYER2`),
    /// `9F05` `HW.SIR` (EAROM write inhibit), `9F06`, `9F07` `MT.BSL`.
    ///
    /// **This latch samples D3, not D0.** `CCastles.v:332` wires
    /// `AutoIncOutput` with `.BD3(BD[3])` where the OUT0 latch at
    /// `CCastles.v:345` gets `.BD0(BD[0])`. The game writes only `0` or `0FF`,
    /// where the two bits agree, so the difference is invisible in practice —
    /// but it is not invisible to a differential harness.
    pub out1: u8,

    /// The bitmap hardware: coordinate window, colour RAM, scroll.
    pub video: Video,

    /// `9800-99FF`, chip 3B — the game's `POKEY0`, and the one it reads
    /// `RANDOM` from (`CG.MAC:40`).
    pub pokey0: Pokey,
    /// `9A00-9BFF`, chip 3D — the game's `POKEY1`.
    pub pokey1: Pokey,

    /// `9400-97FF` — trackball counters and switches.
    pub input: Input,

    pub cycles: u64,
    /// Cycles since the watchdog was last strobed.
    pub watchdog: u32,
    pub watchdog_limit: u32,
    /// Set once the watchdog has gone hungry. Reported, not acted on: a real
    /// board would reset, but silently resetting hides the fault.
    pub watchdog_expired: bool,

    /// Raised by the frame scheduler, cleared by the `INTACK` strobe.
    pub irq_pending: bool,

    // Observability for the boot smoke test.
    pub watchdog_strobes: u64,
    /// Writes to `HW.BSL` (`9E87`), whether or not the bank changed. The game
    /// often writes the bank it is already in — `CST.MAC:315` selects bank 0
    /// when bank 0 is already live — so this counts intent.
    pub bank_writes: u64,
    /// Writes that actually flipped the bank.
    pub bank_switches: u64,

    /// Address of the instruction currently executing, kept up to date through
    /// [`Bus::begin_instruction`]. One store per instruction, always.
    pub current_pc: u16,

    /// Which instruction last wrote each bitmap nibble. `None` unless
    /// [`Machine::enable_write_log`] has been called; nothing is recorded and
    /// nothing is allocated until then.
    ///
    /// Indexed `addr * 2 + parity`, where `parity` is 1 for the high nibble —
    /// the same parity rule [`Video::window_read`] and [`Video::framebuffer`]
    /// use. **Keyed by RAM address, not by screen position**, and that is the
    /// whole design: a store lands at an address, but which pixel of the
    /// picture that address is depends on the scroll registers *at the moment
    /// the picture is read*. Recording screen positions at write time would be
    /// silently wrong the first time the game scrolls.
    /// [`Machine::pixel_writers`] does the projection instead.
    pub writers: Option<Box<[Option<u16>]>>,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    pub fn new() -> Self {
        Machine {
            ram: Box::new([0; 0x8000]),
            sram: Box::new([0; 0x1000]),
            earom: Box::new([0; 0x100]),
            prog: Box::new([0; 0x6000]),
            data: Box::new([0; 0x4000]),
            mob_rom: Box::new([0; 0x4000]),
            motion_roms_loaded: false,
            out0: 0,
            out1: 0,
            video: Video::new(),
            pokey0: Pokey::new(),
            pokey1: Pokey::new(),
            input: Input::new(),
            cycles: 0,
            watchdog: 0,
            watchdog_limit: DEFAULT_WATCHDOG_CYCLES,
            watchdog_expired: false,
            irq_pending: false,
            watchdog_strobes: 0,
            bank_writes: 0,
            bank_switches: 0,
            current_pc: 0,
            writers: None,
        }
    }

    /// Start recording which instruction last wrote each bitmap nibble.
    ///
    /// Off by default. Turning it on allocates the log and costs one store per
    /// bitmap write; it changes nothing the machine computes.
    pub fn enable_write_log(&mut self) {
        self.writers = Some(vec![None; self.ram.len() * 2].into_boxed_slice());
    }

    /// Record that the current instruction wrote one nibble.
    #[inline]
    fn note_nibble(&mut self, addr: usize, high: bool) {
        let pc = self.current_pc;
        if let Some(log) = self.writers.as_mut() {
            log[addr * 2 + high as usize] = Some(pc);
        }
    }

    /// Record that the current instruction wrote a whole byte — both pixels.
    #[inline]
    fn note_byte(&mut self, addr: usize) {
        let pc = self.current_pc;
        if let Some(log) = self.writers.as_mut() {
            log[addr * 2] = Some(pc);
            log[addr * 2 + 1] = Some(pc);
        }
    }

    /// Which instruction last wrote each visible pixel, indexed exactly as
    /// [`Machine::framebuffer`] — row-major, `WIDTH * HEIGHT`.
    ///
    /// `None` if the log is off, and `None` per pixel for anything not written
    /// since it was switched on. The projection uses the scroll registers as
    /// they are *now*, matching what `framebuffer` would return now.
    pub fn pixel_writers(&self) -> Option<Vec<Option<u16>>> {
        let log = self.writers.as_ref()?;
        let mut out = vec![None; WIDTH * HEIGHT];
        for line in 0..HEIGHT {
            let base = self.video.row_for_line(line) as usize * ROW_BYTES;
            for px in 0..WIDTH {
                let x = self.video.hscroll.wrapping_add(px as u8);
                let addr = base + (x >> 1) as usize;
                out[line * WIDTH + px] = log[addr * 2 + (x & 1) as usize];
            }
        }
        Some(out)
    }

    /// Load the two ROM images produced by the Phase 1 assembler.
    pub fn load_roms(&mut self, prog: &[u8], data: &[u8]) -> Result<(), String> {
        if prog.len() != 0x6000 {
            return Err(format!(
                "program image is {} bytes, expected 24576 (A000-FFFF)",
                prog.len()
            ));
        }
        if data.len() != 0x4000 {
            return Err(format!(
                "castle data image is {} bytes, expected 16384 (A000-DFFF)",
                data.len()
            ));
        }
        self.prog.copy_from_slice(prog);
        self.data.copy_from_slice(data);
        Ok(())
    }

    /// Load the motion-object picture ROMs: 16384 bytes, `136022-106.8d`
    /// followed by `136022-107.8b`.
    ///
    /// That is the order `chill65-diff`'s `romset.rs` slices them out of the
    /// corpus file `372BR.RS4`, and the order MAME's `ccastles3` set names
    /// them, so one layout serves the runtime, the ROM-set reconstruction and
    /// the oracle without a translation step anywhere.
    ///
    /// Separate from [`Machine::load_roms`] because these ROMs are not on the
    /// CPU bus and because a machine without them must keep working: leave
    /// this uncalled and motion objects are simply absent.
    pub fn load_motion_roms(&mut self, image: &[u8]) -> Result<(), String> {
        if image.len() != 0x4000 {
            return Err(format!(
                "motion-object image is {} bytes, expected 16384 (8d then 8b)",
                image.len()
            ));
        }
        self.mob_rom.copy_from_slice(image);
        self.motion_roms_loaded = true;
        Ok(())
    }

    /// True when the castle-data bank is mapped at `A000-DFFF`.
    #[inline]
    pub fn data_bank_selected(&self) -> bool {
        self.out0 & 0x80 != 0
    }

    /// Strobe the watchdog, as a write to `9E00` does.
    pub fn pet_watchdog(&mut self) {
        self.watchdog = 0;
        self.watchdog_strobes += 1;
    }

    /// Write one bit of an addressable latch. `data_bit` is the already-selected
    /// data-bus bit in its low position — the two latches sample *different*
    /// bits of the bus, so the choice is made by the caller.
    fn latch_write(latch: &mut u8, addr: u16, data_bit: u8) {
        let mask = 1u8 << (addr & 0x07);
        if data_bit & 1 != 0 {
            *latch |= mask;
        } else {
            *latch &= !mask;
        }
    }

    /// The visible picture, one byte per pixel: the **five-bit colour RAM
    /// address**, 0–31, after bitmap and motion objects have been arbitrated.
    ///
    /// # This is not the bitmap index
    ///
    /// It was, until motion objects existed. `Video::framebuffer` still returns
    /// the raw 4-bit bitmap nibbles and is still the bitmap extraction; this
    /// composites motion objects over them through
    /// [`crate::video::cram_address`], which is the arbitration
    /// `ColorMemory.v:16-21` performs. The result indexes `video.cram`
    /// directly, so a consumer wanting colour does `cram[pixel & 0x1F]` and
    /// nothing else.
    ///
    /// With no picture ROMs loaded every pixel is `MV = 7` — no object — and
    /// the result is exactly `BITMAP_CRAM_BASE + nibble`, which is what the old
    /// contract meant. So a machine without them behaves as it always did, one
    /// constant offset aside.
    pub fn framebuffer(&self) -> Vec<u8> {
        let bitmap = self.video.framebuffer(&self.ram[..]);
        if !self.motion_roms_loaded {
            return bitmap
                .iter()
                .map(|&nibble| (crate::video::BITMAP_CRAM_BASE + nibble as usize) as u8)
                .collect();
        }

        let mut out = Vec::with_capacity(bitmap.len());
        for line in 0..HEIGHT {
            // A line buffer is filled while the vertical counter reads one
            // line and displayed on the next, so the objects visible here were
            // evaluated a line earlier. `FIRST_VISIBLE_LINE` maps the picture's
            // row 0 onto the counter.
            let vc = (line as u32 + crate::frame::FIRST_VISIBLE_LINE)
                .wrapping_sub(crate::motion::DISPLAY_DELAY_LINES as u32) as u8;
            let objects =
                crate::motion::render_line(&self.sram[..], &self.mob_rom[..], self.out1, vc);
            for x in 0..WIDTH {
                let nibble = bitmap[line * WIDTH + x];
                let p = objects[x];
                out.push(crate::video::cram_address(p.mv, p.mpi, nibble) as u8);
            }
        }
        out
    }

    /// A stable identity for the current picture.
    ///
    /// FNV-1a over [`Machine::framebuffer`], so **motion objects are inside the
    /// hash**. They have to be: a hash that ignored half the picture would let
    /// the sprite model be wrong without any comparison noticing.
    pub fn frame_hash(&self) -> u64 {
        crate::video::fnv1a(&self.framebuffer())
    }
}

impl Bus for Machine {
    fn begin_instruction(&mut self, pc: u16) {
        self.current_pc = pc;
    }

    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            // The bitmap data window. `BITMDn` is not qualified by `BRWn`
            // (AddresDecoders.v:74), so a *read* is a bitmap access too, and
            // steps the coordinates exactly as a write does.
            0x0002 => {
                let v = self.video.window_read(&self.ram[..]);
                self.video.auto_step(self.out1);
                v
            }
            // 0000 and 0001 latch coordinates only on write, so reading them
            // is an ordinary RAM read.
            0x0000..=0x7FFF => self.ram[addr as usize],
            0x8000..=0x8FFF => self.sram[(addr - 0x8000) as usize],
            0x9000..=0x93FF => self.earom[(addr & 0xFF) as usize],

            // Inputs, split on BA[9] (CCastles.v:114-117). The LETA sees only
            // BA[1:0], so its four registers mirror every 4 bytes.
            0x9400..=0x95FF => self.input.leta_read(addr as usize & 0x03),
            0x9600..=0x97FF => self.input.switch_byte(),

            // Two POKEYs, split on BA[9] (AudioOutput.v:16-17). Each sees
            // only BA[3:0], so its registers mirror every 16 bytes.
            0x9800..=0x99FF => {
                let c = self.cycles;
                self.pokey0.read(addr as usize & 0x0F, c)
            }
            0x9A00..=0x9BFF => {
                let c = self.cycles;
                self.pokey1.read(addr as usize & 0x0F, c)
            }

            // The control strips are write strobes; reads are not meaningful.
            0x9C00..=0x9FFF => 0xFF,

            0xA000..=0xDFFF => {
                let i = (addr - 0xA000) as usize;
                if self.data_bank_selected() {
                    self.data[i]
                } else {
                    self.prog[i]
                }
            }

            // Never banked — which is why CDB.MAC must live above E000: it
            // switches the window under its own feet.
            0xE000..=0xFFFF => self.prog[(addr - 0xA000) as usize],
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            // Writing 0000/0001 latches a coordinate — *and* still writes the
            // RAM byte. `WE` is satisfied by its ordinary-DRAM term
            // (DynamicRam.v:42, `BITMDn & ~DRAMn`), because these are not
            // bitmap-window accesses. Both effects happen.
            0x0000 => {
                self.video.xcoord = value;
                self.ram[0] = value;
                self.note_byte(0);
            }
            0x0001 => {
                self.video.ycoord = value;
                self.ram[1] = value;
                self.note_byte(1);
            }
            // The window. The write goes to the coordinate address, never to
            // 0002 itself, and may be dropped entirely below row 32.
            0x0002 => {
                // Read the target before the write, since `auto_step` moves the
                // coordinates on immediately afterwards.
                let target = self.video.window_addr() as usize;
                let high = self.video.pixa();
                let landed = self.video.window_write_enabled();
                self.video.window_write(&mut self.ram[..], value);
                if landed {
                    self.note_nibble(target, high);
                }
                self.video.auto_step(self.out1);
            }
            0x0003..=0x7FFF => {
                self.ram[addr as usize] = value;
                self.note_byte(addr as usize);
            }
            0x8000..=0x8FFF => self.sram[(addr - 0x8000) as usize] = value,
            0x9000..=0x93FF => self.earom[(addr & 0xFF) as usize] = value,

            0x9400..=0x97FF => {} // inputs are read-only
            0x9800..=0x99FF => {
                let c = self.cycles;
                self.pokey0.write(addr as usize & 0x0F, value, c)
            }
            0x9A00..=0x9BFF => {
                let c = self.cycles;
                self.pokey1.write(addr as usize & 0x0F, value, c)
            }

            0x9C00..=0x9C7F => {} // UART / EAROM recall-write
            0x9C80..=0x9CFF => self.video.hscroll = value, // HSLD
            0x9D00..=0x9D7F => self.video.vscroll = value, // VSLD
            0x9D80..=0x9DFF => self.irq_pending = false,   // INTACK
            0x9E00..=0x9E7F => self.pet_watchdog(),
            // OUT0 samples D0 (CCastles.v:345).
            0x9E80..=0x9EFF => {
                let before = self.out0;
                Machine::latch_write(&mut self.out0, addr, value & 1);
                if addr & 0x07 == 0x07 {
                    self.bank_writes += 1;
                }
                if (before ^ self.out0) & 0x80 != 0 {
                    self.bank_switches += 1;
                }
            }
            // OUT1 samples D3 (CCastles.v:332) — not D0.
            0x9F00..=0x9F7F => Machine::latch_write(&mut self.out1, addr, (value >> 3) & 1),
            0x9F80..=0x9FFF => self.video.cram_write(addr, value),

            // ROM. Writes are ignored, not an error: the game writes through
            // pointers that sometimes address ROM, and a real board simply
            // does nothing.
            0xA000..=0xFFFF => {}
        }
    }

    fn tick(&mut self, cycles: u8) {
        self.cycles += cycles as u64;
        self.watchdog = self.watchdog.saturating_add(cycles as u32);
        if self.watchdog >= self.watchdog_limit {
            self.watchdog_expired = true;
        }
    }

    /// Inspect without side effects. The strips and POKEY are omitted rather
    /// than read, so a debugger cannot acknowledge an interrupt by looking.
    fn peek(&mut self, addr: u16) -> u8 {
        match addr {
            // Reading the bitmap window steps the coordinates, so inspection
            // must take the value without going through `read`.
            0x0002 => self.video.window_read(&self.ram[..]),
            0x9400..=0x9FFF => 0xFF,
            _ => self.read(addr),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Machine {
        let mut m = Machine::new();
        // Distinguish the banks the way the real images do: the castle data
        // starts with 0x00, the program with the ATARI copyright string.
        m.prog[0] = b' ';
        m.prog[1] = b' ';
        m.prog[2] = b'(';
        m.data[0] = 0x00;
        // A marker up in the fixed region.
        m.prog[0xE000 - 0xA000] = 0xA9;
        m
    }

    /// Set a coordinate, plant a pixel through the window, and stop.
    ///
    /// ```text
    /// 0200  A9 40   LDA #$40     ; row 64 — at or above row 32, so writable
    /// 0202  85 01   STA $01      ; Y coordinate
    /// 0204  A9 10   LDA #$10     ; x = 16, an even column: the low nibble
    /// 0206  85 00   STA $00      ; X coordinate
    /// 0208  A9 50   LDA #$50     ; pixel value 5, in the high nibble
    /// 020A  85 02   STA $02      ; <- the write we expect to be blamed
    /// ```
    const PLANT: [u8; 12] = [
        0xA9, 0x40, 0x85, 0x01, 0xA9, 0x10, 0x85, 0x00, 0xA9, 0x50, 0x85, 0x02,
    ];
    /// Address of the `STA $02` above.
    const PLANT_STORE_PC: u16 = 0x020A;
    /// `{yCoord, xCoord[7:1]}` for row 64, x 16.
    const PLANT_ADDR: usize = (0x40 << 7) | (0x10 >> 1);

    fn run_plant(log: bool) -> (Machine, crate::Cpu) {
        let mut m = Machine::new();
        m.ram[0x0200..0x0200 + PLANT.len()].copy_from_slice(&PLANT);
        if log {
            m.enable_write_log();
        }
        let mut cpu = crate::Cpu::new();
        cpu.pc = 0x0200;
        for _ in 0..6 {
            cpu.step(&mut m).expect("plant program runs");
        }
        (m, cpu)
    }

    #[test]
    fn the_write_log_blames_the_instruction_that_stored_the_pixel() {
        let (m, _) = run_plant(true);

        let log = m.writers.as_ref().expect("log enabled");
        let at = |addr: usize, high: bool| log[addr * 2 + high as usize];

        assert_eq!(
            at(PLANT_ADDR, false),
            Some(PLANT_STORE_PC),
            "the low nibble at {PLANT_ADDR:04X} should be blamed on the STA $02"
        );
        assert_eq!(
            at(PLANT_ADDR, true),
            None,
            "the neighbouring pixel in the same byte was never written"
        );

        // The coordinate stores are attributed to their own instructions.
        assert_eq!(at(1, false), Some(0x0202), "STA $01 wrote ram[1]");
        assert_eq!(at(0, false), Some(0x0206), "STA $00 wrote ram[0]");
    }

    #[test]
    fn the_log_projects_onto_the_same_pixels_as_the_framebuffer() {
        let (m, _) = run_plant(true);

        // Row 64 with the scroll at its floor of 24 is visible line 40, and
        // x 16 with no horizontal scroll is column 16.
        let line = 0x40 - crate::video::VSCROLL_FLOOR as usize;
        let index = line * WIDTH + 16;

        // Nibble 5 with no object over it is colour RAM address 16 + 5.
        assert_eq!(
            m.framebuffer()[index],
            (crate::video::BITMAP_CRAM_BASE + 5) as u8,
            "the pixel we planted"
        );
        let writers = m.pixel_writers().expect("log enabled");
        assert_eq!(writers[index], Some(PLANT_STORE_PC));
        assert_eq!(
            writers[index + 1],
            None,
            "the pixel next door is untouched, not blamed by association"
        );
    }

    #[test]
    fn the_log_is_off_by_default_and_changes_nothing_when_on() {
        let (plain, _) = run_plant(false);
        assert!(plain.writers.is_none(), "off unless asked for");
        assert!(plain.pixel_writers().is_none());

        let (logged, _) = run_plant(true);
        assert_eq!(
            plain.frame_hash(),
            logged.frame_hash(),
            "enabling the log changed the picture"
        );
        assert_eq!(
            plain.cycles, logged.cycles,
            "enabling the log changed the cycle count"
        );
    }

    #[test]
    fn reset_state_selects_the_program_bank() {
        let m = machine();
        assert_eq!(m.out0, 0, "reset clears every latch bit");
        assert!(!m.data_bank_selected(), "the board wakes up in program");
    }

    #[test]
    fn the_bank_latch_switches_reads_of_a000_but_never_e000() {
        let mut m = machine();
        assert_eq!(m.read(0xA000), b' ', "program bank");
        assert_eq!(m.read(0xE000), 0xA9);

        // Select the data bank: write bit 7, which lives at 9E87, with D0 set.
        m.write(0x9E87, 0xFF);
        assert!(m.data_bank_selected());
        assert_eq!(m.read(0xA000), 0x00, "castle data bank");
        assert_eq!(m.read(0xE000), 0xA9, "E000-FFFF is never banked");

        m.write(0x9E87, 0x00);
        assert_eq!(m.read(0xA000), b' ', "back to program");
    }

    #[test]
    fn the_isr_bank_sniff_can_tell_the_banks_apart() {
        // CIN.MAC:35-41 reads A000 and tests for zero, because the latch is
        // write-only. If reads ignored the bank, both would look the same and
        // the ISR would restore the wrong one with nothing failing loudly.
        let mut m = machine();
        let in_program = m.read(0xA000);
        m.write(0x9E87, 0x01);
        let in_data = m.read(0xA000);
        assert_ne!(in_program, 0, "program bank byte must be non-zero");
        assert_eq!(in_data, 0, "data bank byte must be zero");
    }

    #[test]
    fn the_latch_takes_one_bit_per_address_from_d0_only() {
        let mut m = Machine::new();
        // 0xFE has D0 clear, so it must CLEAR the bit despite the other seven
        // bits being set. This is the trap: a byte-wide store would set it.
        m.write(0x9E87, 0xFF);
        assert_eq!(m.out0, 0x80);
        m.write(0x9E87, 0xFE);
        assert_eq!(m.out0, 0x00, "only D0 latches");

        // Each address owns its own bit.
        for bit in 0..8u16 {
            let mut m = Machine::new();
            m.write(0x9E80 + bit, 0x01);
            assert_eq!(m.out0, 1 << bit, "address 9E8{bit:X} drives bit {bit}");
        }

        // The strip is 128 bytes, so the low three bits pick the latch bit and
        // everything between is a mirror.
        let mut m = Machine::new();
        m.write(0x9EF7, 0x01);
        assert_eq!(m.out0, 0x80, "9EF7 mirrors 9E87");
    }

    #[test]
    fn out1_is_a_separate_latch() {
        let mut m = Machine::new();
        m.write(0x9F00, 0x08); // D3 — OUT1's data bit
        assert_eq!(m.out1, 0x01);
        assert_eq!(m.out0, 0x00, "the two latches are independent");
    }

    #[test]
    fn intack_clears_a_pending_interrupt() {
        let mut m = Machine::new();
        m.irq_pending = true;
        m.write(0x9D80, 0x00);
        assert!(!m.irq_pending);
    }

    #[test]
    fn the_watchdog_expires_unless_strobed() {
        let mut m = Machine::new();
        m.watchdog_limit = 1000;
        m.tick(255);
        assert!(!m.watchdog_expired);
        for _ in 0..10 {
            m.tick(255);
        }
        assert!(m.watchdog_expired, "a stuck machine is caught");

        // A healthy one strobes 9E00 and never trips.
        let mut m = Machine::new();
        m.watchdog_limit = 1000;
        for _ in 0..100 {
            m.tick(200);
            m.write(0x9E00, 0x00);
        }
        assert!(!m.watchdog_expired);
        assert_eq!(m.watchdog_strobes, 100);
    }

    #[test]
    fn rom_writes_are_ignored_rather_than_faulting() {
        let mut m = machine();
        m.write(0xE000, 0x00);
        assert_eq!(m.read(0xE000), 0xA9, "ROM is unchanged");
        m.write(0xA000, 0xFF);
        assert_eq!(m.read(0xA000), b' ');
    }

    #[test]
    fn ram_regions_round_trip() {
        let mut m = Machine::new();
        // 0000-0002 are excluded: they are the bitmap coordinate window, not
        // plain RAM. See the video module and the tests below.
        for addr in [0x0003u16, 0x4000, 0x7FFF, 0x8000, 0x8FFF] {
            m.write(addr, 0x5A);
            assert_eq!(m.read(addr), 0x5A, "{addr:04X}");
        }
        // The EAROM is 256 bytes mirrored across its 1K window.
        m.write(0x9000, 0x11);
        assert_eq!(m.read(0x9100), 0x11, "mirrored");
    }

    #[test]
    fn the_input_region_splits_into_trackball_and_switches() {
        // CCastles.v:114-117 splits IN0n on BA[9].
        let mut m = Machine::new();

        // 9600-97FF: switches, active low, so idle reads all ones. Zero here
        // would read as every switch held down at once.
        assert_eq!(m.read(0x9600), 0xFF, "idle switches");
        assert_eq!(m.read(0x97FF), 0xFF, "mirrored across the half");
        m.input.set_switch(crate::input::Switch::CoinLeft, true);
        assert_eq!(m.read(0x9600), 0xFD, "coin-left pressed clears bit 1");

        // 9400-95FF: the LETA counters, which start at zero.
        assert_eq!(m.read(0x9400), 0x00, "vertical");
        m.input.add_trackball_delta(0, 9);
        m.input.latch_frame();
        assert_eq!(m.read(0x9400), 9, "HW.TBV");
        assert_eq!(m.read(0x9401), 0, "HW.TBH untouched");
        // Only BA[1:0] reach the chip, so registers mirror every 4 bytes.
        assert_eq!(m.read(0x9404), 9, "9404 mirrors 9400");
    }

    #[test]
    fn the_coordinate_window_writes_through_to_raw_ram() {
        let mut m = Machine::new();
        // Row 40, column 10 -> byte 40*128 + 5.
        m.write(0x0001, 40); // Y
        m.write(0x0000, 10); // X
        m.write(0x9F00, 0x08); // AXn = 1: disable X auto-step (D3!)
        m.write(0x9F01, 0x08); // AYn = 1: disable Y auto-step
        m.write(0x0002, 0x70);
        assert_eq!(m.ram[40 * 128 + 5], 0x07, "even x lands in the low nibble");
        assert_eq!(m.read(0x0002), 0x70, "and reads back in the high nibble");

        // The neighbouring pixel shares the byte and must survive.
        m.write(0x0000, 11);
        m.write(0x0002, 0xE0);
        assert_eq!(m.ram[40 * 128 + 5], 0xE7);
    }

    #[test]
    fn writing_a_coordinate_also_writes_the_ram_byte() {
        // 0000/0001 are not bitmap-window accesses, so the ordinary DRAM write
        // term in DynamicRam.v:42 is satisfied and both effects happen.
        let mut m = Machine::new();
        m.write(0x0000, 0x33);
        assert_eq!(m.video.xcoord, 0x33);
        assert_eq!(m.ram[0], 0x33, "the RAM byte is written too");
        assert_eq!(m.read(0x0000), 0x33, "and reading 0000 is a plain RAM read");
    }

    #[test]
    fn accessing_the_window_auto_steps_the_coordinates() {
        let mut m = Machine::new();
        m.write(0x0001, 40);
        m.write(0x0000, 0);
        // Reset OUT1 is all-zero, so both axes are enabled and incrementing.
        // Disable Y so the walk stays on one row.
        m.write(0x9F01, 0x08);

        for i in 0..4u8 {
            m.write(0x0002, (i + 1) << 4);
        }
        assert_eq!(m.video.xcoord, 4, "four writes stepped x four times");
        assert_eq!(m.ram[40 * 128], 0x21, "pixels 0 and 1 packed into one byte");
        assert_eq!(m.ram[40 * 128 + 1], 0x43);

        // A read steps the coordinates just as a write does.
        m.write(0x0000, 0);
        let _ = m.read(0x0002);
        assert_eq!(m.video.xcoord, 1);
    }

    #[test]
    fn the_out1_latch_samples_d3_not_d0() {
        // CCastles.v:332 wires .BD3(BD[3]) where OUT0 gets .BD0(BD[0]).
        let mut m = Machine::new();
        m.write(0x9F00, 0x01); // D3 clear, D0 set
        assert_eq!(m.out1, 0x00, "D0 must not reach the OUT1 latch");
        m.write(0x9F00, 0x08); // D3 set
        assert_eq!(m.out1, 0x01);
        // The game only ever writes 0 or 0FF, where both bits agree.
        m.write(0x9F00, 0xFF);
        assert_eq!(m.out1, 0x01);
        m.write(0x9F00, 0x00);
        assert_eq!(m.out1, 0x00);
    }

    #[test]
    fn peek_does_not_step_the_coordinates() {
        let mut m = Machine::new();
        m.write(0x0001, 40);
        m.write(0x0000, 7);
        m.peek(0x0002);
        assert_eq!(m.video.xcoord, 7, "inspection must not move the window");
    }

    #[test]
    fn the_frame_hash_moves_when_the_picture_does() {
        let mut m = Machine::new();
        let before = m.frame_hash();
        m.write(0x0001, 0x20); // a row inside the visible window
        m.write(0x0000, 4);
        m.write(0x0002, 0xF0);
        assert_ne!(m.frame_hash(), before);
        assert_eq!(m.framebuffer().len(), 256 * 232);
    }

    #[test]
    fn the_two_pokeys_are_independent_and_split_on_ba9() {
        let mut m = Machine::new();
        m.write(0x9800, 0x11); // POKEY0 register 0
        m.write(0x9A00, 0x22); // POKEY1 register 0
        assert_eq!(m.pokey0.regs[0], 0x11);
        assert_eq!(m.pokey1.regs[0], 0x22, "BA9 selects the other chip");

        // Each chip sees only BA[3:0], so registers mirror every 16 bytes.
        m.write(0x9810, 0x33);
        assert_eq!(m.pokey0.regs[0], 0x33, "9810 mirrors 9800");
        m.write(0x99F0, 0x44);
        assert_eq!(m.pokey0.regs[0], 0x44, "still POKEY0 at the top of its half");
    }

    #[test]
    fn random_reads_through_the_bus_and_tracks_machine_cycles() {
        let mut m = Machine::new();
        // Release the poly from init, as CST.MAC:18 does.
        m.write(0x980F, 0x07);
        let a = m.read(0x980A);
        m.tick(200);
        let b = m.read(0x980A);
        assert_ne!(a, b, "RANDOM must move with elapsed machine cycles");
    }

    #[test]
    fn peek_does_not_acknowledge_interrupts() {
        let mut m = Machine::new();
        m.irq_pending = true;
        m.peek(0x9D80);
        assert!(m.irq_pending, "inspection must not perturb the machine");
    }

    #[test]
    fn load_roms_checks_the_sizes() {
        let mut m = Machine::new();
        assert!(m.load_roms(&[0; 0x6000], &[0; 0x4000]).is_ok());
        assert!(m.load_roms(&[0; 100], &[0; 0x4000]).is_err());
        assert!(m.load_roms(&[0; 0x6000], &[0; 100]).is_err());
    }

    #[test]
    fn the_cpu_can_run_out_of_this_bus() {
        use crate::Cpu;
        // The real reset sequence: E000 holds a9 00 8d 87 9e — LDA #0 /
        // STA $9E87, CIN.MAC:7's `TRAI 0 HW.BSL ; select bank 0 and jump`.
        let mut m = machine();
        let base = 0xE000 - 0xA000;
        m.prog[base..base + 5].copy_from_slice(&[0xA9, 0x00, 0x8D, 0x87, 0x9E]);
        m.prog[0xFFFC - 0xA000] = 0x00;
        m.prog[0xFFFD - 0xA000] = 0xE0;

        let mut cpu = Cpu::new();
        cpu.reset(&mut m);
        assert_eq!(cpu.pc, 0xE000, "reset vector followed");
        cpu.step(&mut m).unwrap();
        cpu.step(&mut m).unwrap();
        assert!(!m.data_bank_selected(), "bank 0 selected, as the ROM intends");
        assert_eq!(m.cycles, 6, "the bus clocked from the CPU");
    }

    /// A machine without picture ROMs is the machine we already had.
    #[test]
    fn motion_roms_are_optional_and_size_checked() {
        let mut m = Machine::new();
        assert!(!m.motion_roms_loaded, "absent until loaded");
        assert!(m.mob_rom.iter().all(|&b| b == 0));

        assert!(
            m.load_motion_roms(&[0u8; 0x2000]).is_err(),
            "one device is not the pair"
        );
        assert!(!m.motion_roms_loaded, "a rejected image must not count as loaded");

        let mut image = vec![0u8; 0x4000];
        image[0] = 0xA5;
        image[0x2000] = 0x5A;
        m.load_motion_roms(&image).expect("the pair");
        assert!(m.motion_roms_loaded);
        // 8d first, 8b second -- the order romset.rs slices 372BR.RS4.
        assert_eq!(m.mob_rom[0], 0xA5);
        assert_eq!(m.mob_rom[0x2000], 0x5A);
    }

    /// Motion objects reach the picture, and the arbitration decides.
    ///
    /// Synthetic ROMs and a hand-built table: nothing game-derived.
    #[test]
    fn a_planted_sprite_is_composited_through_the_arbitration() {
        use crate::motion::{DISPLAY_DELAY_LINES, OBJECT_BYTES};

        let mut m = Machine::new();

        // A picture ROM that is transparent everywhere except picture 1, row 0,
        // half 0, whose four pixels are colours 2, 7, 2, 2.
        let mut rom = vec![0u8; 0x4000];
        for a in 0..0x2000 {
            rom[a] = 0x0F;
            rom[0x2000 + a] = 0xFF;
        }
        let addr = 1usize << 5; // picture 1, row 0, half 0
        // Four pixels, MSB-first: colour 2, then 7 (transparent), then 2, 2.
        // Encoded rather than hand-computed, because working the plane bits out
        // by hand is exactly how you plant the wrong colour and then debug the
        // renderer for it.
        let (mut p3, mut p2, mut p1) = (0u8, 0u8, 0u8);
        for (i, mv) in [2u8, 7, 2, 2].into_iter().enumerate() {
            let b = 3 - i as u8;
            p3 |= ((mv >> 2) & 1) << b;
            p2 |= ((mv >> 1) & 1) << b;
            p1 |= (mv & 1) << b;
        }
        rom[addr] = p3;
        rom[0x2000 + addr] = (p2 << 4) | p1;
        m.load_motion_roms(&rom).expect("motion roms");

        // Put the object on the first visible line. The buffer is filled a
        // line early, so an object visible on picture row 0 is evaluated at
        // FIRST_VISIBLE_LINE - DISPLAY_DELAY_LINES.
        let vc = (crate::frame::FIRST_VISIBLE_LINE - DISPLAY_DELAY_LINES as u32) as u8;
        let e = 0xE00; // MT.BSL clear
        m.sram[e] = 1; // picture
        m.sram[e + 1] = 0xF0u8.wrapping_sub(vc); // sum lands on 0xF0: row 0
        m.sram[e + 2] = 0x00; // MPI clear
        m.sram[e + 3] = 60; // x
        let _ = OBJECT_BYTES;

        let fb = m.framebuffer();
        // MPI clear, so the object wins: address {0, 0, mv} = mv.
        assert_eq!(fb[60], 2, "sprite pixel should win over the bitmap");
        assert_eq!(fb[62], 2);
        // The transparent pixel leaves the bitmap showing.
        assert_eq!(
            fb[61],
            crate::video::BITMAP_CRAM_BASE as u8,
            "colour 7 is transparent"
        );
        // And somewhere with no object at all is plain bitmap.
        assert_eq!(fb[200], crate::video::BITMAP_CRAM_BASE as u8);

        // Moving the sprite changes the hash, which is the whole point of
        // compositing before hashing.
        let before = m.frame_hash();
        m.sram[e + 3] = 61;
        assert_ne!(m.frame_hash(), before, "a sprite that moves must be visible");
    }

    /// Without picture ROMs the picture is exactly what it always was, one
    /// constant offset aside.
    #[test]
    fn no_motion_roms_means_the_old_contract() {
        let mut m = Machine::new();
        m.ram[0x2000] = 0x35; // two bitmap pixels, nibbles 5 and 3
        assert!(!m.motion_roms_loaded);
        let fb = m.framebuffer();
        let bitmap = m.video.framebuffer(&m.ram[..]);
        assert!(fb
            .iter()
            .zip(&bitmap)
            .all(|(&a, &b)| a as usize == crate::video::BITMAP_CRAM_BASE + b as usize));
    }
}
