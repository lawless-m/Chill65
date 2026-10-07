//! Tempest's board: a 6502, a colour vector generator, a math box, two POKEYs
//! and an EAROM.
//!
//! A third board beside Crystal Castles' and Space Duel's, not a
//! generalisation of either. The [`Bus`] trait is the seam that lets them
//! differ, and `gate2.md` records why that is worth keeping: bending one
//! struct to cover several would make each harder to check against its own
//! evidence.
//!
//! # The map, from `ALCOMN.MAC`'s hardware definitions
//!
//! ```text
//! 0000-07FF  RAM, 2K                       ALTEST.MAC:246-266 clears 00-07
//! 0800-080F  colour RAM, 16 entries        ALCOMN.MAC:241 COLPORT
//! 0C00       IN1, switches and timers      ALCOMN.MAC:243-251
//! 0D00       option switch 0               ALCOMN.MAC:253
//! 0E00       option switch 1               ALCOMN.MAC:254
//! 2000-2FFF  vector RAM, 4K                ALCOMN.MAC:256 VECRAM
//! 3000-3FFF  vector ROM, 4K                ALCOMN.MAC:257 ROMSTART
//! 4000       OUT0 latch                    ALCOMN.MAC:259-264
//! 4800       VG start                      ALCOMN.MAC:266
//! 5000       watchdog *and* interrupt ack  ALCOMN.MAC:267-268
//! 5800       VG reset                      ALCOMN.MAC:269
//! 6000-603F  EAROM data/address            ALCOMN.MAC:276 EADAL
//! 6040       EAROM control (w) / math box status (r)
//! 6050       EAROM read port               ALCOMN.MAC:275 EAIN
//! 6060/6070  math box output low/high      ALCOMN.MAC:288-290
//! 6080-609F  math box registers            ALCOMN.MAC:291-311 MBSTAR
//! 60C0-60CF  POKEY 1                       ALCOMN.MAC:279
//! 60D0-60DF  POKEY 2                       ALCOMN.MAC:280
//! 60E0       OUTANK latch                  ALCOMN.MAC:282-285
//! 9000-DFFF  program ROM, 20K              ALCOMN.MAC:271 PROG
//! ```
//!
//! **UNVERIFIED:** the decode's exact width. Regions are taken at the
//! granularity the source names them, and the aux board is treated as fully
//! decoded within `6000-60FF`.
//!
//! # `5000` is two things at once
//!
//! `ALCOMN.MAC:267-268` reads `WTCHDG =5000` then `INTACK =WTCHDG`, so one
//! write both feeds the watchdog and acknowledges the interrupt. That is not
//! an economy of the source's making: `ALHARD.MAC:56`'s IRQ handler strobes
//! it once, at `SOFTOK`, and relies on both effects. Space Duel keeps the two
//! at separate addresses, which is why this is worth stating rather than
//! copying across.

use crate::bus::Bus;
use crate::mbox::MathBox;
use crate::pokey::Pokey;
use crate::vg::{BeamMove, Segment, StatDecode, Stop, Vg};

/// The 6502 clock.
///
/// **UNVERIFIED**, but corroborated: `ALDIAG.MAC:41-50`'s calibrated software
/// loop implies roughly 1.5 MHz, and 1,512,000 is the figure the same family
/// of Atari vector boards runs at — Space Duel's is measured at exactly this
/// from MAME's driver. The divider chain itself is not established.
pub const CPU_HZ: u32 = 1_512_000;

/// Half-period of the 3 kHz square wave read at `IN1` D7.
///
/// 1,512,000 / 3,000 / 2 = 252 exactly, and an exact division is itself
/// evidence for the clock: `ALTEST.MAC:446-454` times a 7 ms loop against
/// this bit, which only works if the rate is what the source says.
pub const THREE_KHZ_HALF_PERIOD: u64 = (CPU_HZ / 3_000 / 2) as u64;

/// Interrupts per game frame.
///
/// The game says nine. `ALEXEC.MAC:49-52` holds the mainline until `FRTIMR`
/// reaches 9 and then clears it, so a frame is nine interrupts by definition.
pub const IRQS_PER_FRAME: u32 = 9;

/// CPU cycles between interrupts.
///
/// The interrupt is the 3 kHz timer divided by twelve: 252 x 2 x 12 = 6,048,
/// giving 250 Hz. `ALWELG.MAC:580` says the game runs "28 PER SECOND", and
/// nine interrupts per frame at 250 Hz is 27.8 frames a second — which is
/// what a programmer writing that comment would round.
///
/// Worth noting because it is the opposite of Space Duel: `sd.rs` records the
/// divide-by-twelve hypothesis as *dead* for that board, where the measured
/// period is 6,144 rather than 6,048. Here the game's own numbers support it.
///
/// **UNVERIFIED:** the divider chain, as ever. What is pinned is the ratio the
/// game depends on, not the circuit that produces it.
pub const CYCLES_PER_IRQ: u32 = 6_048;

/// CPU cycles in one game frame: 54,432.
pub const CYCLES_PER_FRAME: u32 = CYCLES_PER_IRQ * IRQS_PER_FRAME;

/// The most instructions one strobe may execute.
///
/// A backstop only: the pass detector in [`TeMachine::vg_go`] normally ends a
/// strobe long before this.
pub const VG_BUDGET: u64 = 200_000;

/// Words in the generator's 8K window, for the pass detector's visited set.
const VEC_BYTES_WORDS: usize = 0x1000;

/// Cycles without a strobe before the watchdog fires.
///
/// Sized against the longest gap the game's *own* normal boot takes. The
/// answer turned out to be reassuring: `ALTEST.MAC:283-289` waits roughly
/// 400 ms for the EAROM supply to settle, which is about 605,000 cycles —
/// but `ALTEST.MAC:285` strobes `WTCHDG` inside that loop, so even the
/// longest quiet stretch on the boot path feeds the dog. Anything comfortably
/// above one game frame is therefore safe, and this is about four.
///
/// **UNVERIFIED:** the real period. As with Space Duel, this is a bound
/// chosen so a healthy machine cannot trip it, not a measurement.
pub const DEFAULT_WATCHDOG_CYCLES: u32 = 200_000;

/// Half the visible width, in generator units.
///
/// The counterpart of [`crate::vg::FULL_SCALE_X`], which is Space Duel's, and
/// it has to be its own number because this board's is not the same one. The
/// witness is again a diagnostic the game runs on itself: `ALVROM.MAC:334-342`
/// is `BONDRY`, which `ALVROM.MAC:248` calls "EDGE OF SCREEN".
///
/// ```text
/// BONDRY: CSTAT WHITE      ;SCREEN BOUNDARY
///         SCAL 1,0
/// VORBOX::CNTR
///         VCTR -MX,-MY,0
///         VCTR 2*MX,0,6
///         ...
/// ```
///
/// `MX=500.` and `MY=540.` (`ALVROM.MAC:332-333`, decimal overrides in a
/// `.RADIX 16` file), and `SCAL 1,0` halves, so the box the self-test paints
/// around the screen runs from -250 to +250 and -270 to +270. Running the
/// built image with the test switch on puts the beam on exactly those four
/// numbers, so the decode agrees with the arithmetic.
///
/// The axes are not the same length because the generator's units are not
/// square — the tube is the usual 4:3, and a beam trace carries no aspect
/// ratio precisely so the tube profile can supply it.
///
/// **Attract mode draws well past this**, to about 386 by 314, and that is not
/// a contradiction: Tempest throws its web and its logo at the viewer, and the
/// parts that leave the screen are meant to leave it. The board clips them;
/// so does the glass.
pub const FULL_SCALE_X: i32 = 250;

/// Half the visible height, in generator units. See [`FULL_SCALE_X`].
pub const FULL_SCALE_Y: i32 = 270;

const RAM_LEN: usize = 0x0800;
const COLOR_LEN: usize = 0x10;
const VEC_RAM_LEN: usize = 0x1000;
const VEC_ROM_LEN: usize = 0x1000;
/// `9000-DFFF`, the ten 2K parts `ALEXEC.COM` cuts.
pub const PROG_LEN: usize = 0x5000;
/// `3000-3FFF`, the two 2K vector ROM parts.
pub const VEC_ROM_IMAGE_LEN: usize = 0x1000;

/// `IN1` bit assignments. `ALCOMN.MAC:243-251`.
mod in1 {
    pub const COIN_R: u8 = 0x01;
    pub const COIN_C: u8 = 0x02;
    pub const COIN_L: u8 = 0x04;
    pub const SLAM: u8 = 0x08;
    pub const TEST: u8 = 0x10;
    pub const DIAG: u8 = 0x20;
    pub const HALT: u8 = 0x40;
    pub const TIMER_3KHZ: u8 = 0x80;
}

/// The panel, the spinner and the two option bytes.
///
/// The two pot bytes are read through the POKEYs' `ALLPOT`, which is why the
/// controls live here rather than in the POKEY model: `ALHARD.MAC:63-81`
/// strobes `POTGO`, reads `ALLPOT`, inverts the low nibble and differences it
/// against last frame's to turn an absolute spinner counter into a movement.
#[derive(Clone, Debug)]
pub struct TeInput {
    /// Spinner position, 0-15. `ALCOMN.MAC:343`: POKEY 1 pot D0-D3.
    pub spinner: u8,
    /// `ALCOMN.MAC:341`: 1 if cocktail.
    pub cocktail: bool,
    /// `ALCOMN.MAC:342`, POKEY 1 pot D5.
    pub option4: bool,

    /// `ALCOMN.MAC:334-335`, POKEY 2 pot.
    pub fire: bool,
    pub superzapper: bool,
    /// `ALCOMN.MAC:332-333`.
    pub start1: bool,
    pub start2: bool,
    /// `ALCOMN.MAC:337`, POKEY 2 pot D0-D2.
    pub option13: u8,

    /// Coin senses, `IN1` D0-D2.
    pub coins: u8,
    pub slammed: bool,
    /// `ALTEST.MAC:280-282` reads `IN1 & MTEST` and takes the *normal* boot
    /// when it is non-zero, so the switch is active low: clear means on.
    pub self_test: bool,
    pub diag_pushed: bool,

    /// `0D00` and `0E00`.
    pub option0: u8,
    pub option1: u8,
}

impl TeInput {
    /// Nothing pressed, upright, spinner centred.
    ///
    /// **UNVERIFIED:** that these option bytes select free play.
    /// `ALEXEC.MAC:244-248` grants two credits when `$CMODE & 3` is zero, but
    /// `$CMODE` is `COIN65`'s decode of `0D00` rather than the byte itself.
    /// Attract mode does not depend on it, so it is left at zero and left
    /// marked rather than guessed into a specific value.
    pub fn upright() -> Self {
        TeInput {
            spinner: 0,
            cocktail: false,
            option4: false,
            fire: false,
            superzapper: false,
            start1: false,
            start2: false,
            option13: 0,
            coins: 0,
            slammed: false,
            self_test: false,
            diag_pushed: false,
            option0: 0,
            option1: 0,
        }
    }

    /// POKEY 1's pot byte: spinner in D0-D3, cocktail strap, option 4.
    fn pot1(&self) -> u8 {
        let mut v = self.spinner & 0x0F;
        if self.cocktail {
            v |= 0x10;
        }
        if self.option4 {
            v |= 0x20;
        }
        v
    }

    /// POKEY 2's pot byte: the panel buttons and three option lines.
    fn pot2(&self) -> u8 {
        let mut v = self.option13 & 0x07;
        if self.superzapper {
            v |= 0x08;
        }
        if self.fire {
            v |= 0x10;
        }
        if self.start1 {
            v |= 0x20;
        }
        if self.start2 {
            v |= 0x40;
        }
        v
    }
}

/// The ER-2055, 64 cells of four bits.
///
/// `ALEARO.MAC:26-32` gives the control lines: `EACK=1` clock, `EAC2=2`,
/// `EAC1=4` (inverted on its way to the chip), `EACE=8` chip select, with
/// C1/C2 selecting read (00), write (10) or erase (11).
///
/// Persistence is the embedder's business, as it is for Space Duel: a machine
/// that keeps high scores across runs is a frontend's decision, not the
/// board's.
#[derive(Clone, Debug)]
pub struct Earom {
    pub control: u8,
    /// The cell the last `6000-603F` write addressed.
    pub address: u8,
    /// The byte that write put on the data lines, held until a write-mode
    /// strobe stores it.
    pub data: u8,
    pub cells: [u8; 0x40],
}

/// Control-port bits. `ALEARO.MAC:29-32`.
mod earom {
    /// `EACK`. Recorded for completeness: the model stores on the mode strobe
    /// rather than on a clock edge, because the sequences in `ALEARO.MAC`
    /// always set the mode and the clock from the same handful of writes and
    /// nothing observes an intermediate state.
    #[allow(dead_code)]
    pub const CLOCK: u8 = 1;
    pub const C2: u8 = 2;
    /// Inverted on its way to the chip, per `ALEARO.MAC:30`.
    pub const C1: u8 = 4;
    pub const CHIP_SELECT: u8 = 8;
}

impl Default for Earom {
    fn default() -> Self {
        Earom {
            control: 0,
            address: 0,
            data: 0,
            cells: [0; 0x40],
        }
    }
}

pub struct TeMachine {
    pub ram: [u8; RAM_LEN],
    /// 16 palette entries. `ALDISP.MAC:2352-2366` writes offsets 0-15;
    /// `ALCOMN.MAC:371-382` documents them as active-low four-bit values.
    /// Stored raw — interpretation belongs to a renderer.
    pub color_ram: [u8; COLOR_LEN],
    pub vec_ram: [u8; VEC_RAM_LEN],
    pub vec_rom: [u8; VEC_ROM_LEN],
    pub prog: [u8; PROG_LEN],

    pub vg: Vg,
    pub segments: Vec<Segment>,
    pub trace_beam: bool,
    pub beam: Vec<BeamMove>,
    pub vg_go_strobes: u64,
    pub vg_fault: Option<Stop>,
    /// Whether the last strobe ended by exhausting its budget rather than by
    /// halting — the ordinary case here; see [`TeMachine::vg_go`].
    pub vg_stopped_by_budget: bool,

    pub mbox: MathBox,
    pub pokey0: Pokey,
    pub pokey1: Pokey,
    pub input: TeInput,
    pub earom: Earom,

    /// The `4000` latch: coin counters D0-D2, X invert D3, Y invert D4.
    pub out0: u8,
    /// The `60E0` latch: LED 2 D0, LED 1 D1, flip D2.
    pub outank: u8,

    pub watchdog: u32,
    pub watchdog_limit: u32,
    pub watchdog_expired: bool,
    pub watchdog_strobes: u64,
    pub irq_pending: bool,

    pub cycles: u64,
}

impl Default for TeMachine {
    fn default() -> Self {
        TeMachine::new()
    }
}

impl TeMachine {
    pub fn new() -> Self {
        let mut vg = Vg::new();
        // The colour board reads its status words differently; see
        // [`StatDecode`].
        vg.stat_decode = StatDecode::ColorSelect;
        TeMachine {
            ram: [0; RAM_LEN],
            color_ram: [0; COLOR_LEN],
            vec_ram: [0; VEC_RAM_LEN],
            vec_rom: [0; VEC_ROM_LEN],
            prog: [0; PROG_LEN],
            vg,
            segments: Vec::new(),
            trace_beam: false,
            beam: Vec::new(),
            vg_go_strobes: 0,
            vg_fault: None,
            vg_stopped_by_budget: false,
            mbox: MathBox::new(),
            pokey0: Pokey::new(),
            pokey1: Pokey::new(),
            input: TeInput::upright(),
            earom: Earom::default(),
            out0: 0,
            outank: 0,
            watchdog: 0,
            watchdog_limit: DEFAULT_WATCHDOG_CYCLES,
            watchdog_expired: false,
            watchdog_strobes: 0,
            irq_pending: false,
            cycles: 0,
        }
    }

    /// Load the two images `chill65_asm::te::images` produces.
    pub fn load_roms(&mut self, vec_rom: &[u8], program: &[u8]) -> Result<(), String> {
        if vec_rom.len() != VEC_ROM_IMAGE_LEN {
            return Err(format!(
                "vector ROM is {} bytes, expected {VEC_ROM_IMAGE_LEN} (3000-3FFF)",
                vec_rom.len()
            ));
        }
        if program.len() != PROG_LEN {
            return Err(format!(
                "program image is {} bytes, expected {PROG_LEN} (9000-DFFF)",
                program.len()
            ));
        }
        self.vec_rom.copy_from_slice(vec_rom);
        self.prog.copy_from_slice(program);
        Ok(())
    }

    /// The generator's 8K window: vector RAM then vector ROM.
    fn vector_window(&self) -> Vec<u8> {
        let mut mem = Vec::with_capacity(VEC_RAM_LEN + VEC_ROM_LEN);
        mem.extend_from_slice(&self.vec_ram);
        mem.extend_from_slice(&self.vec_rom);
        mem
    }

    /// Run the display list, as a write to `4800` does.
    ///
    /// Runs to completion inside the strobe rather than in step with the CPU —
    /// the same documented simplification `SdMachine::vg_go` makes. The CPU
    /// observes the finished state through `IN1`'s halt bit, and
    /// `ALHARD.MAC:170-175` restarts the generator whenever it sees it set.
    fn vg_go(&mut self) {
        let mem = self.vector_window();
        self.vg.reset(0);
        self.segments.clear();
        if self.trace_beam {
            self.beam.clear();
        }

        // One pass, not one budget.
        //
        // The list loops, so running it to a budget draws the same shapes over
        // and over — measured at 76,000 segments a frame for a picture of
        // about 200. What the phosphor shows is the *union* of those passes,
        // which is one pass, so that is what is collected here.
        //
        // A pass ends when the generator returns to a word it has already
        // executed at the top level. Depth matters: a glyph subroutine is
        // legitimately called many times in one list — `ALPHA` emits a `JSRL`
        // per character — so only top-level revisits mean the list has come
        // round again.
        let mut seen = vec![false; VEC_BYTES_WORDS];
        let mut stop = Stop::Budget;
        let mut steps = 0u64;
        while steps < VG_BUDGET {
            if self.vg.depth() == 0 {
                let pc = self.vg.pc as usize;
                if seen[pc] {
                    break;
                }
                seen[pc] = true;
            }
            let step = if self.trace_beam {
                self.vg
                    .step_trace(&mem, &mut self.segments, &mut self.beam)
            } else {
                self.vg.step(&mem, &mut self.segments)
            };
            steps += 1;
            if let Some(s) = step {
                stop = s;
                break;
            }
        }
        // `Stop::Budget` is **not** a fault on this board, where it is one on
        // Space Duel's.
        //
        // Space Duel's lists halt themselves, so a list still running when the
        // budget expires is a runaway. Tempest's do not: `VGHALT` is called
        // only by the self-test — `ALTEST.MAC:486`, "PLACE HALT AT END OF
        // DISPLAY LIST" — and the main display path never places one. Its
        // lists end in a `JMPL`, double-buffered through the `JMPAHI`/`JMPALO`
        // words the display code patches, so the generator draws them round
        // and round until the CPU stops it. `ALHARD.MAC:170-175` is the other
        // half of that arrangement: every interrupt the handler tests `IN1`'s
        // halt bit and, finding the generator stopped, strobes `VGSTOP` then
        // `VGSTART` to set it going again.
        //
        // So on this machine a list that runs out of budget is the ordinary
        // case, and the budget stands in for the `VGSTOP` the CPU would have
        // issued. A `StackOverflow` is still a fault — the hardware has five
        // levels and no more.
        if matches!(stop, Stop::StackOverflow) && self.vg_fault.is_none() {
            self.vg_fault = Some(stop);
        }
        self.vg_stopped_by_budget = matches!(stop, Stop::Budget);
        self.vg.halted = true;
        self.vg_go_strobes += 1;
    }

    /// Strobe the watchdog *and* acknowledge the interrupt: one write to
    /// `5000` does both, because `ALCOMN.MAC:268` makes `INTACK` the same
    /// address.
    pub fn pet_watchdog(&mut self) {
        self.watchdog = 0;
        self.watchdog_strobes += 1;
        self.irq_pending = false;
    }

    /// `IN1`. `ALCOMN.MAC:243-251`.
    fn in1(&self) -> u8 {
        let mut v = 0u8;
        if (self.cycles / THREE_KHZ_HALF_PERIOD) & 1 == 1 {
            v |= in1::TIMER_3KHZ;
        }
        if self.vg.halted {
            v |= in1::HALT;
        }
        if !self.input.diag_pushed {
            v |= in1::DIAG;
        }
        // Active low: `ALTEST.MAC:280-282` takes the normal boot when set.
        if !self.input.self_test {
            v |= in1::TEST;
        }
        if !self.input.slammed {
            v |= in1::SLAM;
        }
        // Low-true, as Space Duel's are: a set bit is no coin.
        v |= (!self.input.coins) & (in1::COIN_R | in1::COIN_C | in1::COIN_L);
        v
    }

    /// A stable identity for the current picture.
    ///
    /// The colour RAM is hashed alongside the segments, and that is not
    /// decoration: `ALDISP.MAC:1078-1100` animates the palette by rewriting
    /// `COLPORT` every frame, so a frame whose geometry is unchanged but whose
    /// colours moved *is* a different picture. Leave the palette out and the
    /// attract test's wedge detection goes blind to exactly the animation
    /// Tempest is famous for.
    pub fn frame_hash(&self) -> u64 {
        let mut bytes = Vec::with_capacity(self.segments.len() * 20 + COLOR_LEN);
        for s in &self.segments {
            for v in [s.x0, s.y0, s.x1, s.y1] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            bytes.push(s.intensity);
            bytes.push(s.color);
        }
        bytes.extend_from_slice(&self.color_ram);
        crate::video::fnv1a(&bytes)
    }
}

impl Bus for TeMachine {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x07FF => self.ram[addr as usize],
            0x0C00..=0x0CFF => self.in1(),
            0x0D00..=0x0DFF => self.input.option0,
            0x0E00..=0x0EFF => self.input.option1,
            0x2000..=0x2FFF => self.vec_ram[(addr - 0x2000) as usize],
            0x3000..=0x3FFF => self.vec_rom[(addr - 0x3000) as usize],
            0x6040..=0x604F => self.mbox.stat(),
            0x6050..=0x605F => self.earom.cells[(self.earom.address & 0x3F) as usize],
            0x6060..=0x606F => self.mbox.ylow(),
            0x6070..=0x607F => self.mbox.yhigh(),
            // The pot lines are live inputs, not settings: `ALHARD.MAC:63-81`
            // strobes `POTGO` and reads `ALLPOT` in the same interrupt, and
            // the low nibble it takes from POKEY 1 is the spinner's current
            // position. So they are refreshed on the way past rather than
            // latched by a setter, which is what Space Duel's option bytes
            // can afford to be.
            0x60C0..=0x60CF => {
                self.pokey0.pot_lines = self.input.pot1();
                self.pokey0.read((addr & 0x0F) as usize, self.cycles)
            }
            0x60D0..=0x60DF => {
                self.pokey1.pot_lines = self.input.pot2();
                self.pokey1.read((addr & 0x0F) as usize, self.cycles)
            }
            0x9000..=0xDFFF => self.prog[(addr - 0x9000) as usize],
            // The 6502's vectors live at DFFA-DFFF in the linked image
            // (`ALHARD.MAC:184`), so the top page must answer for them.
            //
            // UNVERIFIED: the real decode. This mirrors the last ROM page.
            0xE000..=0xFFFF => self.prog[PROG_LEN - 0x1000 + (addr as usize & 0x0FFF)],
            _ => 0xFF,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x07FF => self.ram[addr as usize] = value,
            0x0800..=0x080F => self.color_ram[(addr & 0x0F) as usize] = value,
            0x2000..=0x2FFF => self.vec_ram[(addr - 0x2000) as usize] = value,
            0x4000..=0x47FF => self.out0 = value,
            0x4800..=0x4FFF => self.vg_go(),
            0x5000..=0x57FF => self.pet_watchdog(),
            0x5800..=0x5FFF => self.vg.halted = true,
            // EAROM address and data. `ALCOMN.MAC:276` calls `6000` the write
            // *base address*: the cell is chosen by the low address bits, so
            // `STA X,EADAL` selects cell X and puts A on the data lines.
            //
            // Both are **latched, not stored**. The read sequence at
            // `ALEARO.MAC:215-224` selects its cell with exactly this write —
            // `LDA #EACE / STA EACTL / STA X,EADAL` — where A holds the
            // control byte and has nothing to do with the contents. Storing on
            // this write would mean every read destroyed the cell it was
            // reading, which is precisely what `EARCND` counted before this was
            // corrected.
            0x6000..=0x603F => {
                self.earom.address = (addr & 0x3F) as u8;
                self.earom.data = value;
            }
            // The control port decides what the latches do. `ALEARO.MAC:26-32`
            // gives the truth table: C1/C2 select read (00), write (10) or
            // erase (11), with `EACE` the chip select. The write path sets
            // write mode *after* placing the data (`ALEARO.MAC:210-211`), so
            // the store happens here.
            0x6040..=0x604F => {
                self.earom.control = value;
                if value & earom::CHIP_SELECT != 0 && value & earom::C1 != 0 {
                    let cell = (self.earom.address & 0x3F) as usize;
                    self.earom.cells[cell] = if value & earom::C2 != 0 {
                        0 // erase
                    } else {
                        self.earom.data
                    };
                }
            }
            0x6080..=0x609F => self.mbox.write((addr & 0x1F) as u8, value),
            0x60C0..=0x60CF => self.pokey0.write((addr & 0x0F) as usize, value, self.cycles),
            0x60D0..=0x60DF => self.pokey1.write((addr & 0x0F) as usize, value, self.cycles),
            0x60E0..=0x60EF => self.outank = value,
            // ROM and everything unmapped. Writes are dropped.
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

    /// Read without side effects: the POKEYs and the strobes are why this
    /// exists.
    fn peek(&mut self, addr: u16) -> u8 {
        match addr {
            0x60C0..=0x60DF => 0xFF,
            _ => self.read(addr),
        }
    }
}

use crate::cpu::{Cpu, CpuError};
use crate::frame::FrameStats;

/// Run one game frame: nine interrupt intervals.
///
/// Deadlines are absolute against the machine's cycle counter, as
/// [`crate::frame::run_frame`] and `run_frame_sd` both do it, so an
/// instruction overrunning an interval steals from that interval alone.
///
/// `irq_pending` is a **latched level**: it stays set until the game writes
/// `5000`, which on this board is the watchdog strobe as well.
pub fn run_frame_te(cpu: &mut Cpu, m: &mut TeMachine) -> Result<FrameStats, CpuError> {
    let start = m.cycles;
    let mut stats = FrameStats::default();

    for interval in 0..IRQS_PER_FRAME {
        m.irq_pending = true;
        stats.irqs_raised += 1;

        let deadline = start + ((interval + 1) as u64) * CYCLES_PER_IRQ as u64;
        while m.cycles < deadline {
            if m.irq_pending && !cpu.interrupt_disable {
                cpu.irq(m);
                stats.irqs_taken += 1;
                continue;
            }
            cpu.step(m)?;
            stats.interpreted += 1;
        }
    }

    stats.cycles = m.cycles - start;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> TeMachine {
        let mut m = TeMachine::new();
        m.load_roms(&[0u8; VEC_ROM_IMAGE_LEN], &[0u8; PROG_LEN])
            .unwrap();
        m
    }

    fn put_words(m: &mut TeMachine, at: usize, words: &[u16]) {
        for (i, w) in words.iter().enumerate() {
            m.vec_ram[(at + i) * 2] = *w as u8;
            m.vec_ram[(at + i) * 2 + 1] = (*w >> 8) as u8;
        }
    }

    const HALT: u16 = 0x2000;

    #[test]
    fn ram_and_vector_ram_round_trip_but_rom_does_not() {
        let mut m = machine();
        m.write(0x0000, 0x11);
        m.write(0x07FF, 0x22);
        assert_eq!((m.read(0x0000), m.read(0x07FF)), (0x11, 0x22));

        m.write(0x2000, 0x33);
        m.write(0x2FFF, 0x44);
        assert_eq!((m.read(0x2000), m.read(0x2FFF)), (0x33, 0x44));

        m.write(0x3000, 0xFF);
        m.write(0x9000, 0xFF);
        assert_eq!((m.read(0x3000), m.read(0x9000)), (0, 0));
    }

    #[test]
    fn load_roms_places_the_images_and_rejects_wrong_sizes() {
        let mut m = TeMachine::new();
        let mut vrom = vec![0u8; VEC_ROM_IMAGE_LEN];
        let mut prog = vec![0u8; PROG_LEN];
        vrom[0] = 0xA1;
        vrom[VEC_ROM_IMAGE_LEN - 1] = 0xA2;
        prog[0] = 0xB1;
        prog[PROG_LEN - 1] = 0xB2;
        m.load_roms(&vrom, &prog).unwrap();

        assert_eq!((m.read(0x3000), m.read(0x3FFF)), (0xA1, 0xA2));
        assert_eq!((m.read(0x9000), m.read(0xDFFF)), (0xB1, 0xB2));

        assert!(m.load_roms(&[0u8; 4], &prog).is_err());
        assert!(m.load_roms(&vrom, &[0u8; 4]).is_err());
    }

    #[test]
    fn the_vectors_planted_at_dffa_answer_at_fffa() {
        // ALHARD.MAC:184's `.VCTRS 0DFFA,IRQ,RESET,IRQ`.
        let mut m = TeMachine::new();
        let mut prog = vec![0u8; PROG_LEN];
        for (i, b) in [0x04, 0xD7, 0x3F, 0xD9, 0x04, 0xD7].iter().enumerate() {
            prog[PROG_LEN - 6 + i] = *b;
        }
        m.load_roms(&[0u8; VEC_ROM_IMAGE_LEN], &prog).unwrap();

        assert_eq!((m.read(0xFFFC), m.read(0xFFFD)), (0x3F, 0xD9), "reset");
        assert_eq!((m.read(0xFFFE), m.read(0xFFFF)), (0x04, 0xD7), "irq");
    }

    #[test]
    fn in1_idles_with_the_switches_released() {
        let mut m = machine();
        let v = m.read(0x0C00);
        assert_eq!(v & in1::TEST, in1::TEST, "self-test is active low");
        assert_eq!(v & in1::SLAM, in1::SLAM, "not slammed");
        assert_eq!(v & in1::DIAG, in1::DIAG, "diagnostic not pushed");
        assert_eq!(v & 0x07, 0x07, "no coins, low-true");

        m.input.self_test = true;
        assert_eq!(m.read(0x0C00) & in1::TEST, 0, "holding it pulls the bit low");
    }

    #[test]
    fn the_halt_bit_follows_the_generator() {
        let mut m = machine();
        assert_eq!(m.read(0x0C00) & in1::HALT, in1::HALT, "idle is halted");
        put_words(&mut m, 0, &[HALT]);
        m.write(0x4800, 0);
        assert_eq!(m.read(0x0C00) & in1::HALT, in1::HALT, "still halted after");
        m.vg.halted = false;
        assert_eq!(m.read(0x0C00) & in1::HALT, 0);
    }

    #[test]
    fn the_three_khz_bit_alternates_on_its_half_period() {
        let mut m = machine();
        let first = m.read(0x0C00) & in1::TIMER_3KHZ;
        m.cycles = THREE_KHZ_HALF_PERIOD;
        assert_ne!(m.read(0x0C00) & in1::TIMER_3KHZ, first, "flipped");
        m.cycles = THREE_KHZ_HALF_PERIOD * 2;
        assert_eq!(m.read(0x0C00) & in1::TIMER_3KHZ, first, "and back");
    }

    #[test]
    fn writing_5000_acknowledges_the_interrupt_and_feeds_the_dog() {
        // ALCOMN.MAC:267-268 — one address, both jobs.
        let mut m = machine();
        m.irq_pending = true;
        m.tick(255);
        assert!(m.watchdog > 0);

        m.write(0x5000, 0);
        assert!(!m.irq_pending, "INTACK");
        assert_eq!(m.watchdog, 0, "and the watchdog");
        assert_eq!(m.watchdog_strobes, 1);
    }

    #[test]
    fn starvation_trips_the_watchdog() {
        let mut m = machine();
        for _ in 0..(DEFAULT_WATCHDOG_CYCLES / 200 + 2) {
            m.tick(200);
        }
        assert!(m.watchdog_expired);
    }

    #[test]
    fn colour_ram_writes_change_the_frame_hash() {
        // The palette is part of the picture: ALDISP.MAC:1078-1100 animates it
        // every frame, and the attract test's only wedge signal is the hash.
        let mut m = machine();
        let before = m.frame_hash();
        m.write(0x0800, 0x0A);
        assert_ne!(m.frame_hash(), before, "a palette change is a new picture");
        assert_eq!(m.color_ram[0], 0x0A);

        m.write(0x080F, 0x05);
        assert_eq!(m.color_ram[15], 0x05, "all sixteen entries");
    }

    #[test]
    fn vgstart_runs_the_list_and_vgstop_halts_it() {
        let mut m = machine();
        put_words(&mut m, 0, &[HALT]);
        m.write(0x4800, 0);
        assert_eq!(m.vg_go_strobes, 1);
        assert!(m.vg.halted);
        assert!(m.vg_fault.is_none());

        m.vg.halted = false;
        m.write(0x5800, 0);
        assert!(m.vg.halted, "5800 resets the generator");
    }

    #[test]
    fn a_looping_list_ends_its_pass_and_is_not_a_fault() {
        // The ordinary case on this board: the display list ends in a JMPL and
        // goes round until the CPU stops it. One pass is the picture.
        let mut m = machine();
        // Draw something, then jump back to the top for ever.
        put_words(&mut m, 0, &[0x4000 + (7 << 5) + 4, 0xE000]);
        m.write(0x4800, 0);

        assert!(m.vg_fault.is_none(), "a loop is not a fault here");
        assert_eq!(m.segments.len(), 1, "one pass, not many");
        assert!(m.vg.halted, "the CPU sees a finished frame");
    }

    #[test]
    fn a_glyph_called_twice_is_drawn_twice() {
        // The pass detector must not mistake a repeated subroutine for the
        // list coming round: `ALPHA` emits one JSRL per character, so the same
        // glyph is legitimately entered many times in one list.
        let mut m = machine();
        let glyph = 0x40; // word address of the shared subroutine
        put_words(
            &mut m,
            0,
            &[0xA000 + glyph, 0xA000 + glyph, 0xA000 + glyph, HALT],
        );
        put_words(&mut m, glyph as usize, &[0x4000 + (7 << 5) + 4, 0xC000]);
        m.write(0x4800, 0);

        assert!(m.vg_fault.is_none());
        assert_eq!(m.segments.len(), 3, "three calls, three vectors");
    }

    #[test]
    fn nesting_past_the_hardware_stack_is_still_a_fault() {
        let mut m = machine();
        // Six levels of JSRL, one deeper than the hardware's five.
        put_words(&mut m, 0, &[0xA000 + 0x10]);
        for level in 0..6u16 {
            let at = 0x10 + level;
            put_words(&mut m, at as usize, &[0xA000 + at + 1]);
        }
        m.write(0x4800, 0);
        assert_eq!(m.vg_fault, Some(Stop::StackOverflow));
    }

    #[test]
    fn the_math_box_answers_through_the_bus() {
        // The self-test's own vector, driven at the addresses ALCOMN gives.
        let mut m = machine();
        m.write(0x6080 + 0x15, 0x34); // XPL
        m.write(0x6080 + 0x0D, 0x34); // ZLL
        m.write(0x6080 + 0x16, 0x12); // XPH
        m.write(0x6080 + 0x0E, 0x12); // ZLH
        m.write(0x6080 + 0x0F, 0);
        m.write(0x6080 + 0x10, 0);
        m.write(0x6080 + 0x0C, 0x10); // N
        m.write(0x6080 + 0x14, 0); // start Z/X'
        assert_eq!(m.read(0x6040) & 0x80, 0, "never busy");
        assert_eq!((m.read(0x6060), m.read(0x6070)), (1, 0), "quotient 1");
    }

    #[test]
    fn the_earom_stores_on_a_write_strobe_and_not_before() {
        // `ALEARO.MAC:210-211`: the data goes out on the address port, and the
        // control port is set to write mode afterwards. Until that happens,
        // nothing is stored.
        let mut m = machine();
        m.write(0x6000 + 5, 0x0C);
        assert_eq!(m.earom.address, 5);
        assert_eq!(m.earom.cells[5], 0, "latched, not yet stored");

        m.write(0x6040, earom::CHIP_SELECT | earom::C1);
        assert_eq!(m.earom.cells[5], 0x0C, "write mode stores it");
        assert_eq!(m.read(0x6050), 0x0C, "and the read port returns it");
    }

    #[test]
    fn reading_a_cell_does_not_destroy_it() {
        // The regression the game itself caught. `ALEARO.MAC:215-224` selects
        // the cell to read with `STA X,EADAL`, where A holds the *control*
        // byte — so a model that stored on that write would overwrite every
        // cell as it read it, and `EARCND` counted exactly that.
        let mut m = machine();
        m.write(0x6000 + 9, 0x0A);
        m.write(0x6040, earom::CHIP_SELECT | earom::C1);
        assert_eq!(m.earom.cells[9], 0x0A);

        // Now the read sequence, verbatim in shape.
        m.write(0x6040, earom::CHIP_SELECT);
        m.write(0x6000 + 9, earom::CHIP_SELECT);
        m.write(0x6040, earom::CHIP_SELECT | earom::CLOCK);
        m.write(0x6040, earom::CHIP_SELECT);
        assert_eq!(m.read(0x6050), 0x0A, "the cell survived being read");
    }

    #[test]
    fn erase_mode_clears_the_addressed_cell() {
        // `ALEARO.MAC:26-28`: C1 and C2 together are erase.
        let mut m = machine();
        m.write(0x6000 + 3, 0x0F);
        m.write(0x6040, earom::CHIP_SELECT | earom::C1);
        assert_eq!(m.earom.cells[3], 0x0F);

        m.write(0x6040, earom::CHIP_SELECT | earom::C1 | earom::C2);
        assert_eq!(m.earom.cells[3], 0, "erased");
    }

    #[test]
    fn peek_never_strobes_anything() {
        let mut m = machine();
        m.peek(0x4800);
        m.peek(0x5000);
        m.peek(0x5800);
        assert_eq!(m.vg_go_strobes, 0);
        assert_eq!(m.watchdog_strobes, 0);
        assert_eq!(m.peek(0x60C0), 0xFF, "POKEY reads are inert");
    }

    /// Nine interrupts a frame, latched, with no drift.
    #[test]
    fn the_frame_raises_nine_interrupts_and_does_not_drift() {
        let mut m = machine();
        // An ISR that acknowledges through 5000 and returns.
        let isr: [u8; 4] = [0x8D, 0x00, 0x50, 0x40]; // STA 5000 ; RTI
        let base = 0xD000 - 0x9000;
        m.prog[base..base + isr.len()].copy_from_slice(&isr);
        // Main loop: enable interrupts, then spin. The `CLI` matters — the
        // 6502 leaves reset with `I` set, and the game clears it at
        // `ALTEST.MAC:292` once the boot sequence is done.
        m.prog[0] = 0x58; // CLI
        m.prog[1] = 0x4C; // JMP 9001
        m.prog[2] = 0x01;
        m.prog[3] = 0x90;
        // Vectors: reset -> 9000, irq -> D000.
        let v = PROG_LEN - 6;
        m.prog[v + 2] = 0x00;
        m.prog[v + 3] = 0x90;
        m.prog[v + 4] = 0x00;
        m.prog[v + 5] = 0xD0;

        let mut cpu = Cpu::new();
        cpu.reset(&mut m);

        const FRAMES: u64 = 20;
        let mut total = 0u64;
        for _ in 0..FRAMES {
            let s = run_frame_te(&mut cpu, &mut m).expect("frame");
            assert_eq!(s.irqs_raised, IRQS_PER_FRAME);
            assert_eq!(s.irqs_taken, IRQS_PER_FRAME, "each one is taken");
            // A frame is never short, and never long by more than the
            // instruction that crossed the last deadline. Absolute deadlines
            // are what stop that residue compounding: whatever a frame
            // overruns by, the next one's intervals are measured from where it
            // actually ended, so the error stays bounded instead of growing.
            let over = s.cycles - CYCLES_PER_FRAME as u64;
            assert!(over <= 7, "frame ran {over} cycles past its deadline");
            total += s.cycles;
        }
        let slip = total - FRAMES * CYCLES_PER_FRAME as u64;
        assert!(
            slip <= 7,
            "twenty frames slipped {slip} cycles; the residue is compounding"
        );
        assert!(!m.watchdog_expired, "the ISR fed the dog");
    }

    #[test]
    fn two_identically_driven_machines_stay_identical() {
        let run = || {
            let mut m = machine();
            m.prog[0] = 0x4C;
            m.prog[1] = 0x00;
            m.prog[2] = 0x90;
            let v = PROG_LEN - 6;
            m.prog[v + 2] = 0x00;
            m.prog[v + 3] = 0x90;
            let mut cpu = Cpu::new();
            cpu.reset(&mut m);
            let mut hashes = Vec::new();
            for _ in 0..5 {
                run_frame_te(&mut cpu, &mut m).expect("frame");
                hashes.push((m.cycles, m.frame_hash()));
            }
            hashes
        };
        assert_eq!(run(), run(), "determinism, cycle for cycle");
    }
}
