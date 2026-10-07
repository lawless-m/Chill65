//! The Atari auto-normalizing vector generator — Space Duel's display hardware.
//!
//! Crystal Castles draws into a bitmap ([`crate::video`]). Space Duel does not
//! have a bitmap at all. The CPU builds a **display list** in vector RAM and
//! strobes the vector generator, which executes that list as a small
//! instruction set and deflects a beam. This module is that instruction set.
//!
//! # Where the encoding comes from
//!
//! Not from a datasheet and not from another emulator: from Atari's own macros,
//! which ship in the game's source tree. `space-duel/VGMC.MAC` is titled
//! "GENERAL PURPOSE MACROS TO FACILITATE PROGRAMMING THE AUTO-NORMALIZING
//! VECTOR GENERATOR" and gives every opcode as a `.WORD` expression. That is
//! the authority here, the way the MiSTer RTL was the authority for Crystal
//! Castles. `space-duel/VGUTR2.MAC` is the game's own run-time library that
//! builds the same words dynamically, and it corroborates each one.
//!
//! Anything those two files do not determine is marked **UNVERIFIED**, as in
//! `hardware.md` — a guess recorded as fact would poison the differential
//! harness later.
//!
//! # Words, and where they live
//!
//! Instructions are 16-bit words, little-endian in CPU memory. `VGUTR2.MAC:282`
//! (`VGADD2`) stores the accumulator first and calls it `;LSB BYTE`, then the X
//! register as `;MSB BYTE`.
//!
//! `VGMC.MAC:36-39` says the addressable instruction space is **8K bytes**, and
//! `JSRL`/`JMPL` hold `<LABL&^H1FFF>/2` — a byte offset masked to 8K, halved.
//! So the generator addresses **4K words**, and `AS2DEC.MAC:64` (`VECRAM=2000`)
//! pins the base:
//!
//! ```text
//! word w  <->  CPU address 0x2000 + 2*w        w = 0x000 .. 0xFFF
//!
//! CPU 2000-27FF   vector RAM   the display list the CPU builds
//! CPU 2800-3FFF   vector ROM   fixed shapes and glyphs
//! ```
//!
//! [`Vg::run`] therefore takes one contiguous `&[u8]` of 0x2000 bytes covering
//! CPU `2000-3FFF`. That is deliberately a plain slice rather than a trait or a
//! closure: it is exactly the address space the hardware sees, it costs no copy
//! when the machine keeps RAM and ROM adjacent, and it cannot accidentally read
//! anything else.
//!
//! # The instruction set
//!
//! Class is the top three bits, except that `011` splits on bit 12:
//!
//! ```text
//! 000  0000-1FFF  VCTR long    two words          VGMC.MAC:126
//! 001  2000       HALT                            VGMC.MAC:31-33
//! 010  4000-5FFF  VCTR short   one word           VGMC.MAC:122
//! 011  6000-6FFF  STAT/COLOR   intensity, colour  VGMC.MAC:64,146
//! 011  7000-7FFF  SCAL         scale              VGMC.MAC:77
//! 100  8040       CNTR         centre the beam    VGMC.MAC:24
//! 101  A000-AFFF  JSRL         call               VGMC.MAC:43
//! 110  C000       RTSL         return             VGMC.MAC:87
//! 111  E000-EFFF  JMPL         jump               VGMC.MAC:98
//! ```
//!
//! ## Vectors
//!
//! The long form is two words, `.WORD DY&^H1FFF,<ZZ*^H2000>+<DX&^H1FFF>` —
//! 13-bit two's-complement deltas, intensity in the second word's top three
//! bits. Note the order: **DY first**.
//!
//! The short form packs both deltas into one word:
//!
//! ```text
//! 4000 + <ZZ*20> + <DX/2 & 1F> + <DY*80 & 1F00>
//!
//! bits 0-4    DX/2, 5-bit two's complement
//! bits 5-7    ZZ
//! bits 8-12   DY/2, 5-bit two's complement
//! ```
//!
//! Both deltas are stored **halved**, so decoding doubles them. The assembler
//! only picks this form when `<|DX| ! |DY|> & 0FFE1` is zero (`VGMC.MAC:121`) —
//! that is, both deltas even and under 32 — which is why halving is lossless.
//!
//! ## Intensity
//!
//! `VGUTR2.MAC:67-72` documents the rule:
//!
//! > `VGBRIT` ... ITS VALUES ARE 0,10,20,30,40,...F0 WHERE 0 IS OFF AND F0 IS
//! > MAX BRIGHTNESS. IN THE VECTOR INSTRUCTIONS ONLY THE UPPER 3 BITS IS USED.
//! > IF Z=1 IN THE VECTOR INSTRUCTIONS THEN THE LAST NON-ZERO Z IS USED.
//!
//! So `ZZ = 0` draws nothing — the beam moves blanked — and `ZZ = 1` means
//! "use the latched intensity". Everything else is the brightness itself.
//!
//! The two scales reconcile exactly. `VGBRIT` is a byte whose *upper three
//! bits* are used, and a STAT word carries `VGBRIT`'s high nibble in bits 4-7
//! (`Z*^H10`, `VGMC.MAC:64`). So the latched three-bit value is the word's
//! bits 4-7 shifted right once, which puts it on the same 0-7 scale as `ZZ`.
//! That is derived, not assumed.
//!
//! **UNVERIFIED:** whether an explicit non-zero `ZZ` on a vector also updates
//! the latch. The phrase "last non-zero Z" could be read that way, but
//! `VGUTR2.MAC:255-260` provides `VGSETZ` specifically to set "the holding
//! buffer for Z intensity" with a `60xx` word, which suggests the latch is
//! written by the status class alone. This model latches only from the status
//! class. **UNVERIFIED:** how a 0-7 brightness maps to a real beam current.
//!
//! ## Status, and the window that Space Duel never uses
//!
//! `STAT` (`VGMC.MAC:55-65`) carries three window-control bits above the
//! intensity: `0100` in/out, `0200` hi/low, and `0400` set when the macro's
//! optional arguments are absent. Together they drive a hardware clipping
//! window.
//!
//! Space Duel does not use it. `VGUTR2.MAC:248-250` is explicit —
//! "FOR THIS GAME ORA 64 INSTEAD OF 60 / SO NEVER USE WINDOW CIRCUIT" — so
//! every status word the game builds through `VGSTAT` has bit `0400` set and
//! the window disabled. This model records the bits and implements no clipping.
//! **UNVERIFIED:** the window's exact geometry, deliberately unmodelled.
//!
//! Note that `64` is not universal: `VGSETZ` at `VGUTR2.MAC:259` builds `60xx`
//! to set the intensity latch without disabling the window. Both are the same
//! class and decode identically here.
//!
//! The colour hardware reuses the class. `COLOR COL,LUM` (`VGMC.MAC:145-147`)
//! emits `<LUM*^H10>+COL` as the low byte and `64` as the high byte, so colour
//! sits in bits 0-3 underneath the intensity.
//!
//! ## Scale
//!
//! `SCAL S,LS` is `7000 + LS + <S*100>` (`VGMC.MAC:77`), where `S` is a binary
//! scale — 0 full size, 1 half, up to 7 = 1/128 — and `LS` is a linear scale
//! with 0 full size, `80` half, `FF` 1/256 (`VGMC.MAC:67-71`).
//!
//! Those ratios pin the *meaning*. They do not pin the arithmetic, so this
//! model applies `delta * (0x100 - linear) / 0x100 >> binary` and marks the
//! rounding and the order of the two steps **UNVERIFIED**.
//!
//! ## Control flow
//!
//! `JSRL` pushes and `RTSL` pops. `VGMC.MAC:82-83`: "THE VG IS CAPABLE OF FIVE
//! LEVELS OF SUBROUTINE CALLS". A sixth call is a fault, not a panic — a
//! display list is data, and malformed data must not be able to take down the
//! runtime. `RTSL` at the top level ends execution, which is how a list
//! entered by `JSRL` terminates when run standalone.
//!
//! `CNTR` is `8040` (`VGMC.MAC:24`), and `VGUTR2.MAC:269-270` builds the same
//! word from `40` and `80` with the low byte commented ";TIMER + SCALE".
//! **UNVERIFIED:** what those low bits do. This model centres the beam and
//! ignores them.

/// One drawn line. Blanked moves (`ZZ = 0`) change the beam position without
/// producing a segment, so a segment always has visible intensity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    /// Brightness on the 0-7 scale the encoding uses. Never 0.
    pub intensity: u8,
    /// Colour index from the status class, 0-15.
    pub color: u8,
}

/// One movement of the beam, lit or not.
///
/// [`Segment`] is what the picture is made of; this is what the *beam* did. A
/// blanked move deposits no light but still takes the beam somewhere, and still
/// takes time doing it, so anything modelling the display rather than the
/// picture needs both. `intensity == 0` is blanked travel, and `color` is then
/// simply whatever was latched.
///
/// Lit moves correspond one-to-one with the [`Segment`]s the same execution
/// produces, field for field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeamMove {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    /// Brightness on the 0-7 scale. 0 means blanked travel.
    pub intensity: u8,
    /// Latched colour index, 0-15. Meaningless when blanked.
    pub color: u8,
    /// A `CNTR` recentre: the position jumps rather than sweeping.
    pub jump: bool,
}

impl BeamMove {
    /// How long this move occupies the beam, in generator ticks.
    ///
    /// **UNVERIFIED**, and the module's most speculative number. `VGMC.MAC` and
    /// `VGUTR2.MAC` describe what to draw and never what it costs; `sd.rs:99`
    /// records the same gap from the machine's side. The model is the Chebyshev
    /// length — `max(|dx|, |dy|)` — on the reasoning that both axes are driven
    /// from one shared clock count, so the larger component sets the duration
    /// and the smaller merely runs at a lower rate. `CNTR` dumps the position
    /// rather than sweeping to it, so a jump costs nothing.
    ///
    /// A uniform rate cancels out of a phosphor model's overall brightness; it
    /// is the *relative* cost of one move against another that this decides.
    pub fn ticks(&self) -> u32 {
        if self.jump {
            return 0;
        }
        (self.x1 - self.x0).unsigned_abs().max((self.y1 - self.y0).unsigned_abs())
    }
}

/// Why [`Vg::run`] stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// A `HALT` instruction. The normal end of a frame's list.
    Halt,
    /// `RTSL` with an empty stack — the end of a list entered by `JSRL`.
    Returned,
    /// The instruction budget ran out: the list loops or is not a list.
    Budget,
    /// A sixth nested `JSRL`, beyond the hardware's five levels.
    StackOverflow,
}

impl Stop {
    /// Whether this is an orderly end. `Budget` and `StackOverflow` are faults.
    pub fn is_clean(self) -> bool {
        matches!(self, Stop::Halt | Stop::Returned)
    }
}

/// Five levels of subroutine nesting — `VGMC.MAC:82-83`.
pub const STACK_DEPTH: usize = 5;

/// Half the visible width, in the generator's own units after scaling.
///
/// The board's **own diagnostic** settles this. `XYSIG.MAC` — "COLOR XY
/// SIGNITURE ANALASIS", written for the Space Duel XY colour graphics board —
/// tests the long-vector instruction by driving the beam from the centre to a
/// corner, at `XYSIG.MAC:143-145`:
///
/// ```text
/// TEST2:  CNTR
///         SCAL 1
///         VCTR 512., 384., 1
/// ```
///
/// `SCAL 1` is binary scale 1, which halves (`VGMC.MAC:77`), so the deltas
/// drawn are 256 and 192 — and a diagnostic drawing a corner is drawing the
/// edge of the deflection it has. The trailing dots are MACRO-11 decimal
/// overrides; the file is in `.RADIX 16` around them.
///
/// The game's own use agrees independently. `AST2RD.MAC:575-576` gives
/// `XRIGHT =20` and `YTOP =18` — hex, so 32 by 24 — as the edges of its
/// playfield, and `AST2RD.MAC:1920` wraps a coordinate with `AND I,XRIGHT-1`,
/// which only makes sense if that span is the whole screen. 512/32 and 384/24
/// are both exactly 16 generator units per playfield unit, and 512:384 is 4:3.
///
/// **UNVERIFIED:** what the hardware does *past* this. Overscan, the DAC's own
/// clip limits and where the tube's edge actually falls are analogue questions
/// the source cannot answer. This is the deflection the software drives, which
/// is what a trace needs; it is not proof that nothing exists beyond it.
pub const FULL_SCALE_X: i32 = 256;

/// Half the visible height, in generator units. See [`FULL_SCALE_X`].
pub const FULL_SCALE_Y: i32 = 192;

/// The vector memory window is CPU `2000-3FFF`: 8K bytes, 4K words.
pub const VEC_BYTES: usize = 0x2000;

/// CPU address the window starts at — `AS2DEC.MAC:64`, `VECRAM=2000`.
pub const VEC_BASE: u16 = 0x2000;

/// The exponent in [`intensity_drive`]. See there.
pub const INTENSITY_GAMMA: f32 = 2.2;

/// A 0-7 intensity code as radiant drive in linear light, 1.0 at full.
///
/// This is what a beam trace wants: not the code, and not a voltage, but light
/// per unit time, so that energy is drive multiplied by dwell.
///
/// # What the corpus settles
///
/// The code is three bits, and that is corroborated twice.
/// `VGUTR2.MAC:67-72`: "`VGBRIT` ... ITS VALUES ARE 0,10,20,30,40,...F0 WHERE 0
/// IS OFF AND F0 IS MAX BRIGHTNESS. IN THE VECTOR INSTRUCTIONS ONLY THE UPPER 3
/// BITS IS USED." And `AST2RD.MAC:272` labels the variable "VECTOR BRIGHTNESS
/// (0=OFF, F0=MAX, 20 INC)" — increments of `20` hex, which is exactly eight
/// steps and so exactly the three-bit field. The game uses `.BRITE =5` as its
/// default (`AST2RD.MAC:642`) and `ORA #0E0 ;FULL BRIGHT ON FLASH`
/// (`AST2RD.MAC:731`), `0E0` being code 7.
///
/// # What the corpus does not settle, and the reasoning that fills it
///
/// Nothing in the source says what a code does to a beam. The chain it goes
/// through is a small weighted-resistor DAC into a Z amplifier into a gun, and
/// two things are true of that chain in general: the DAC is linear in
/// **voltage** by construction, and a CRT gun is not linear in **light** — its
/// output follows a power law of drive. So this models
/// `(code/7)^INTENSITY_GAMMA`, with the exponent at 2.2, the usual figure for
/// a CRT.
///
/// **UNVERIFIED, and reasoned rather than measured.** It is the right *shape*
/// on general principles; the exponent is a starting value and the real
/// Space Duel Z chain — an Atari colour XY board driving a Wells-Gardner 6100
/// — has not been measured. Jed Margolin's "The Secret Life of Vector
/// Generators" covers the X and Y integrators and not the Z path, so it does
/// not help here. Correcting this needs a board and a photometer, not a
/// listing.
pub fn intensity_drive(code: u8) -> f32 {
    let code = code & 0x07;
    if code == 0 {
        return 0.0;
    }
    (code as f32 / 7.0).powf(INTENSITY_GAMMA)
}

/// The same, for the colour board, whose Z code is four bits wide.
///
/// [`StatDecode::ColorSelect`] keeps a status word's whole nibble rather than
/// its top three bits, so a [`BeamMove`]'s intensity arrives on one of *two*
/// scales depending on which instruction set it:
///
/// - **A status word** carries `VGBRIT` whole. `ALVGUT.MAC:218` documents
///   `VGSTAT`'s entry condition as "(Y)=VECTOR BRIGHTNESS (0,10,20,...,F0)",
///   which is a nibble, 0-15.
/// - **A vector word's `ZZ` field** is three bits and carries `VGBRIT`'s upper
///   three. `ALVGUT.MAC:332` says exactly that — "(VGBRIT)=VECTOR BRIGHTNESS
///   (ONLY THE UPPER 3 BITS ARE USED)" — and `ALVGUT.MAC:349` EORs them into
///   the word's high bits.
///
/// A `ZZ` code is therefore the nibble shifted right one, and this shifts it
/// back before the same power law [`intensity_drive`] reasons out is applied.
/// The running game corroborates: over 200 attract frames its beam carries
/// codes 5, 6, 7 and 12 and nothing else, and the two common ones — 6 and 12 —
/// are one `VGBRIT` of `C0` arriving by the two routes.
///
/// **UNVERIFIED: where the two ranges meet.** A status word *could* set a
/// nibble below 8, and it would be indistinguishable here from a `ZZ` code and
/// read as the dimmer of the two. Tempest never writes one — that needs a
/// `VGBRIT` below `80`, and none appears.
///
/// **UNVERIFIED, inherited:** whether the colour board's Z DAC is four bits at
/// all. See [`StatDecode::ColorSelect`], which keeps the fourth because it can
/// be dropped later and cannot be recovered.
pub fn intensity_drive_color(code: u8) -> f32 {
    let nibble = if code >= 8 { code } else { code << 1 };
    if nibble == 0 {
        return 0.0;
    }
    (nibble as f32 / 15.0).powf(INTENSITY_GAMMA)
}

/// Sign-extend an `n`-bit two's-complement field.
fn sign_extend(value: u16, bits: u32) -> i32 {
    let shift = 32 - bits;
    ((value as i32) << shift) >> shift
}

/// How a status word carries colour and intensity.
///
/// The instruction set is not in question: `VGMC.MAC` is byte-identical
/// between the Space Duel and Tempest corpora, so every opcode, the stack, the
/// word addressing and the `ZZ` rules are shared. The status word is the one
/// place two boards disagree, and they disagree because one drives a
/// monochrome tube and the other a colour one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum StatDecode {
    /// One word carries both fields, which is what Space Duel writes.
    ///
    /// `VGUTR2.MAC:248-250`'s `VGSTAT` always ORs `64` into the high byte, so
    /// bit 11 is clear in every status word that board emits and there is
    /// nothing for it to select.
    #[default]
    Combined,
    /// Bit 11 selects *which* field the word carries, which is what Tempest
    /// needs.
    ///
    /// `ALVGUT.MAC:224`'s `VGSTAT` ORs `60` into the high byte and the caller
    /// supplies the distinguishing bit: `ALDISP.MAC:34-35` defines
    /// `MZCOLO=8` — "NEW COLOR STAT BIT MASK" — against `MZBRIT=0`. So a
    /// colour word is `68xx` and an intensity word `60xx`, and
    /// `ALDISP.MAC:1182-1185` issues one of each back to back, which is the
    /// proof the bit selects rather than combines: if a single word could
    /// carry both there would be no reason to write two.
    ///
    /// It matters because the two fields occupy overlapping bits, so under
    /// [`Self::Combined`] each word clobbers the other's latch.
    /// `ALVROM.MAC:161-163`'s `CSTAT` emits `68C0+colour`, whose low byte has
    /// bits 6 and 7 set: read as an intensity field that is `(0x68C0 >> 5) & 7`
    /// — a bright 6 — so every colour change would also rewrite the intensity.
    /// Read the other way an intensity word's `F0` gives `0xF0 & 0x0F`, so
    /// every intensity change would reset the colour to 0. Selecting keeps the
    /// game's two-word sequence meaning what it says.
    ///
    /// **UNVERIFIED:** what the `C0` in a colour word's low byte means. It is
    /// constant across every `CSTAT` in the picture ROM, so it is carried in
    /// [`Vg::window_bits`]' company rather than acted on.
    ColorSelect,
}

#[derive(Clone, Debug)]
pub struct Vg {
    /// Word address, 0x000-0xFFF.
    pub pc: u16,
    stack: [u16; STACK_DEPTH],
    depth: usize,
    /// Binary scale, 0-7. Larger is smaller.
    pub binary_scale: u8,
    /// Linear scale, 0-255. 0 is full size.
    pub linear_scale: u8,
    /// Latched intensity on the 0-7 scale, used when a vector's `ZZ` is 1.
    pub intensity: u8,
    /// Latched colour index, 0-15.
    pub color: u8,
    /// Window-control bits from the status class, recorded but not acted on.
    pub window_bits: u16,
    /// How the status word's colour and intensity fields are read.
    pub stat_decode: StatDecode,
    pub x: i32,
    pub y: i32,
    pub halted: bool,
    /// Instructions executed since the last [`Vg::reset`].
    pub executed: u64,
}

impl Default for Vg {
    fn default() -> Self {
        Vg::new()
    }
}

impl Vg {
    pub fn new() -> Self {
        Vg {
            pc: 0,
            stack: [0; STACK_DEPTH],
            depth: 0,
            binary_scale: 0,
            linear_scale: 0,
            intensity: 0,
            color: 0,
            window_bits: 0,
            stat_decode: StatDecode::Combined,
            x: 0,
            y: 0,
            halted: true,
            executed: 0,
        }
    }

    /// Clear position, stack and counters, and point at word `start`.
    ///
    /// Scale, intensity and colour are *not* cleared: they are latches, and
    /// nothing in the sources says a start strobe resets them.
    pub fn reset(&mut self, start: u16) {
        self.pc = start & 0x0FFF;
        self.depth = 0;
        self.x = 0;
        self.y = 0;
        self.halted = false;
        self.executed = 0;
    }

    /// Current subroutine nesting depth, 0 at the top level.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Read one word from the vector window. `mem` covers CPU `2000-3FFF`.
    fn word(mem: &[u8], w: u16) -> u16 {
        let i = (w as usize & 0x0FFF) * 2;
        u16::from_le_bytes([mem[i], mem[i + 1]])
    }

    /// Apply both scales to a delta.
    ///
    /// See the module note: the ratios are from `VGMC.MAC:67-71`, the exact
    /// arithmetic is UNVERIFIED.
    fn scale(&self, delta: i32) -> i32 {
        let linear = delta * (0x100 - self.linear_scale as i32) / 0x100;
        linear >> self.binary_scale
    }

    /// Move the beam by a scaled delta, emitting a segment when lit.
    ///
    /// `trace`, when present, receives the move whether it is lit or not.
    fn vector(
        &mut self,
        dx: i32,
        dy: i32,
        zz: u8,
        out: &mut Vec<Segment>,
        trace: Option<&mut Vec<BeamMove>>,
    ) {
        let (x0, y0) = (self.x, self.y);
        self.x += self.scale(dx);
        self.y += self.scale(dy);
        // ZZ = 0 is a blanked move. ZZ = 1 defers to the latch.
        // VGUTR2.MAC:70-72.
        let intensity = match zz {
            0 => 0,
            1 => self.intensity,
            other => other,
        };
        if let Some(trace) = trace {
            trace.push(BeamMove {
                x0,
                y0,
                x1: self.x,
                y1: self.y,
                intensity,
                color: self.color,
                jump: false,
            });
        }
        if intensity == 0 {
            return;
        }
        out.push(Segment {
            x0,
            y0,
            x1: self.x,
            y1: self.y,
            intensity,
            color: self.color,
        });
    }

    /// Execute one instruction. Returns `Some(stop)` when execution ends.
    ///
    /// `mem` must be [`VEC_BYTES`] long and cover CPU `2000-3FFF`.
    pub fn step(&mut self, mem: &[u8], out: &mut Vec<Segment>) -> Option<Stop> {
        self.step_inner(mem, out, None)
    }

    /// As [`Vg::step`], and also records the beam's movement in `trace`.
    pub fn step_trace(
        &mut self,
        mem: &[u8],
        out: &mut Vec<Segment>,
        trace: &mut Vec<BeamMove>,
    ) -> Option<Stop> {
        self.step_inner(mem, out, Some(trace))
    }

    /// The one implementation. `step` and `step_trace` differ only in whether
    /// they are watching.
    fn step_inner(
        &mut self,
        mem: &[u8],
        out: &mut Vec<Segment>,
        mut trace: Option<&mut Vec<BeamMove>>,
    ) -> Option<Stop> {
        debug_assert_eq!(mem.len(), VEC_BYTES, "vector window is 8K");
        let w = Self::word(mem, self.pc);
        self.pc = (self.pc + 1) & 0x0FFF;
        self.executed += 1;

        match w >> 13 {
            // Long vector: DY first, then ZZ and DX. VGMC.MAC:126.
            0b000 => {
                let w2 = Self::word(mem, self.pc);
                self.pc = (self.pc + 1) & 0x0FFF;
                let dy = sign_extend(w & 0x1FFF, 13);
                let dx = sign_extend(w2 & 0x1FFF, 13);
                let zz = ((w2 >> 13) & 0x7) as u8;
                self.vector(dx, dy, zz, out, trace.as_deref_mut());
            }
            0b001 => {
                self.halted = true;
                return Some(Stop::Halt);
            }
            // Short vector: both deltas halved. VGMC.MAC:122.
            0b010 => {
                let dx = sign_extend(w & 0x1F, 5) * 2;
                let dy = sign_extend((w >> 8) & 0x1F, 5) * 2;
                let zz = ((w >> 5) & 0x7) as u8;
                self.vector(dx, dy, zz, out, trace.as_deref_mut());
            }
            0b011 => {
                if w & 0x1000 == 0 {
                    // Status / colour. VGMC.MAC:64, VGMC.MAC:146.
                    match self.stat_decode {
                        StatDecode::Combined => {
                            // Bits 4-7 hold VGBRIT's high nibble; the hardware
                            // uses VGBRIT's top three bits, one place further
                            // right.
                            self.intensity = ((w >> 5) & 0x7) as u8;
                            self.color = (w & 0x0F) as u8;
                        }
                        // Bit 11 says which field this word carries, and the
                        // other latch is left alone — see [`StatDecode`].
                        StatDecode::ColorSelect if w & 0x0800 != 0 => {
                            self.color = (w & 0x0F) as u8;
                        }
                        StatDecode::ColorSelect => {
                            // `VGBRIT` counts 00,10,20..F0 (ALDISP.MAC:1184),
                            // so the whole nibble is kept rather than the top
                            // three bits the monochrome board uses.
                            //
                            // **UNVERIFIED:** whether the colour board's Z DAC
                            // is three bits or four. Keeping the fourth loses
                            // nothing if it turns out to be three, and cannot
                            // be recovered if it is dropped.
                            self.intensity = ((w >> 4) & 0xF) as u8;
                        }
                    }
                    // Bit 11 is the selector under `ColorSelect` and must not
                    // be folded in here; the mask stops at 0x0700 regardless.
                    self.window_bits = w & 0x0700;
                } else {
                    // Scale. VGMC.MAC:77.
                    self.binary_scale = ((w >> 8) & 0x7) as u8;
                    self.linear_scale = (w & 0xFF) as u8;
                }
            }
            // Centre. VGMC.MAC:24; low bits UNVERIFIED, ignored.
            0b100 => {
                let (x0, y0) = (self.x, self.y);
                self.x = 0;
                self.y = 0;
                if let Some(trace) = trace.as_deref_mut() {
                    trace.push(BeamMove {
                        x0,
                        y0,
                        x1: 0,
                        y1: 0,
                        intensity: 0,
                        color: self.color,
                        jump: true,
                    });
                }
            }
            // Call. VGMC.MAC:43.
            0b101 => {
                if self.depth == STACK_DEPTH {
                    return Some(Stop::StackOverflow);
                }
                self.stack[self.depth] = self.pc;
                self.depth += 1;
                self.pc = w & 0x0FFF;
            }
            // Return. VGMC.MAC:87. At the top level this ends the list.
            0b110 => {
                if self.depth == 0 {
                    return Some(Stop::Returned);
                }
                self.depth -= 1;
                self.pc = self.stack[self.depth];
            }
            // Jump. VGMC.MAC:98.
            0b111 => self.pc = w & 0x0FFF,
            _ => unreachable!("three bits"),
        }
        None
    }

    /// Execute until the list ends or `limit` instructions have run.
    ///
    /// A runaway list yields [`Stop::Budget`] rather than looping forever: the
    /// list is data, and malformed data must not hang the runtime.
    pub fn run(&mut self, mem: &[u8], limit: u64, out: &mut Vec<Segment>) -> Stop {
        let deadline = self.executed + limit;
        while self.executed < deadline {
            if let Some(stop) = self.step(mem, out) {
                return stop;
            }
        }
        Stop::Budget
    }

    /// As [`Vg::run`], and also records every beam movement in `trace`.
    pub fn run_trace(
        &mut self,
        mem: &[u8],
        limit: u64,
        out: &mut Vec<Segment>,
        trace: &mut Vec<BeamMove>,
    ) -> Stop {
        let deadline = self.executed + limit;
        while self.executed < deadline {
            if let Some(stop) = self.step_trace(mem, out, trace) {
                return stop;
            }
        }
        Stop::Budget
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a vector window from words starting at word 0.
    fn mem(words: &[u16]) -> Vec<u8> {
        let mut m = vec![0u8; VEC_BYTES];
        for (i, w) in words.iter().enumerate() {
            m[i * 2] = *w as u8;
            m[i * 2 + 1] = (*w >> 8) as u8;
        }
        m
    }

    /// Place words at a given word address, for call targets.
    fn put(m: &mut [u8], at: u16, words: &[u16]) {
        for (i, w) in words.iter().enumerate() {
            let o = (at as usize + i) * 2;
            m[o] = *w as u8;
            m[o + 1] = (*w >> 8) as u8;
        }
    }

    /// The encodings under test are computed here from the VGMC.MAC formulas
    /// rather than copied from the game, so the tests check the decoder against
    /// the specification and not against themselves.
    fn vctr_short(dx: i32, dy: i32, zz: u8) -> u16 {
        0x4000
            + ((zz as u16) << 5)
            + ((dx / 2) as u16 & 0x1F)
            + (((dy as u16) << 7) & 0x1F00)
    }
    fn vctr_long(dx: i32, dy: i32, zz: u8) -> [u16; 2] {
        [
            (dy as u16) & 0x1FFF,
            ((zz as u16) << 13) + ((dx as u16) & 0x1FFF),
        ]
    }
    const HALT: u16 = 0x2000;
    const RTSL: u16 = 0xC000;
    const CNTR: u16 = 0x8040;
    fn jsrl(w: u16) -> u16 {
        0xA000 + w
    }
    fn jmpl(w: u16) -> u16 {
        0xE000 + w
    }
    fn color(col: u8, lum: u8) -> u16 {
        0x6400 | ((lum as u16) << 4) | col as u16
    }
    fn scal(s: u8, ls: u8) -> u16 {
        0x7000 + ((s as u16) << 8) + ls as u16
    }

    fn run(words: &[u16]) -> (Vg, Vec<Segment>, Stop) {
        let m = mem(words);
        let mut vg = Vg::new();
        vg.reset(0);
        let mut out = Vec::new();
        let stop = vg.run(&m, 10_000, &mut out);
        (vg, out, stop)
    }

    #[test]
    fn halt_stops_and_sets_the_flag() {
        let (vg, out, stop) = run(&[HALT]);
        assert_eq!(stop, Stop::Halt);
        assert!(vg.halted);
        assert!(out.is_empty());
        assert_eq!(vg.executed, 1);
    }

    #[test]
    fn a_short_vector_draws_and_moves_the_beam() {
        let (vg, out, stop) = run(&[vctr_short(8, 4, 7), HALT]);
        assert_eq!(stop, Stop::Halt);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0],
            Segment { x0: 0, y0: 0, x1: 8, y1: 4, intensity: 7, color: 0 }
        );
        assert_eq!((vg.x, vg.y), (8, 4));
    }

    #[test]
    fn short_vector_fields_sign_extend() {
        // -2 halves to -1, which is 0x1F in five bits — the case that fails
        // outright if the field is read as unsigned.
        let w = vctr_short(-2, -4, 4);
        assert_eq!(w & 0x1F, 0x1F, "dx field is the 5-bit two's complement");
        let (vg, out, _) = run(&[w, HALT]);
        assert_eq!((out[0].x1, out[0].y1), (-2, -4));
        assert_eq!((vg.x, vg.y), (-2, -4));
    }

    #[test]
    fn a_long_vector_carries_thirteen_bit_deltas() {
        // Beyond the short form's reach in both directions, and odd, so the
        // assembler could not have chosen the short form here.
        let v = vctr_long(-1023, 511, 5);
        let (vg, out, _) = run(&[v[0], v[1], HALT]);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].x1, out[0].y1), (-1023, 511));
        assert_eq!(out[0].intensity, 5);
        assert_eq!((vg.x, vg.y), (-1023, 511));
    }

    #[test]
    fn the_long_form_puts_dy_in_the_first_word() {
        // The order is easy to get backwards and nothing else would catch it:
        // a symmetric test vector would pass either way.
        let v = vctr_long(0, 100, 7);
        assert_eq!(v[0], 100, "first word is DY");
        let (_, out, _) = run(&[v[0], v[1], HALT]);
        assert_eq!((out[0].x1, out[0].y1), (0, 100));
    }

    #[test]
    fn zero_intensity_moves_without_drawing() {
        let (vg, out, _) = run(&[vctr_short(10, 0, 0), vctr_short(0, 6, 7), HALT]);
        assert_eq!(out.len(), 1, "the blanked move draws nothing");
        assert_eq!(
            out[0],
            Segment { x0: 10, y0: 0, x1: 10, y1: 6, intensity: 7, color: 0 },
            "but it still moved the beam"
        );
        assert_eq!((vg.x, vg.y), (10, 6));
    }

    #[test]
    fn intensity_one_uses_the_latched_value() {
        // VGUTR2.MAC:71-72. COLOR's LUM sits in bits 4-7; the latch keeps the
        // top three of VGBRIT, one place right, so LUM=12 latches 6.
        let (vg, out, _) = run(&[color(3, 12), vctr_short(4, 0, 1), HALT]);
        assert_eq!(vg.intensity, 6);
        assert_eq!(out[0].intensity, 6, "ZZ=1 defers to the latch");
        assert_eq!(out[0].color, 3);
        // The control: an explicit ZZ overrides it.
        let (_, out2, _) = run(&[color(3, 12), vctr_short(4, 0, 7), HALT]);
        assert_eq!(out2[0].intensity, 7);
    }

    #[test]
    fn a_latched_intensity_of_zero_draws_nothing() {
        let (_, out, _) = run(&[color(5, 0), vctr_short(4, 0, 1), HALT]);
        assert!(out.is_empty(), "ZZ=1 with nothing latched stays dark");
    }

    #[test]
    fn the_status_class_records_window_bits_without_clipping() {
        // A STAT word with the window enabled — hi/low set, in/out clear.
        // Space Duel never builds one (VGUTR2.MAC:248-250), but the decode
        // must not mistake those bits for intensity or colour.
        let stat = 0x6000 + 0x200 + (4 << 4);
        let (vg, out, _) = run(&[stat, vctr_short(6, 0, 1), HALT]);
        assert_eq!(vg.window_bits, 0x200);
        assert_eq!(vg.intensity, 2, "bits 4-7 shifted one right");
        assert_eq!(out.len(), 1, "no clipping is applied");
    }

    /// Run with Tempest's decode selected.
    fn run_color(words: &[u16]) -> (Vg, Vec<Segment>, Stop) {
        let m = mem(words);
        let mut vg = Vg::new();
        vg.stat_decode = StatDecode::ColorSelect;
        vg.reset(0);
        let mut out = Vec::new();
        let stop = vg.run(&m, 10_000, &mut out);
        (vg, out, stop)
    }

    #[test]
    fn color_select_takes_one_field_per_word() {
        // The two words `ALDISP.MAC:1182-1185` writes back to back, built the
        // way the game builds them: `ALVROM.MAC:161-163`'s CSTAT emits
        // `68C0+colour`, and `VGSTA1` pairs `MZBRIT=0` with a `VGBRIT` value
        // counting 00,10,20..F0.
        let cstat = |c: u16| 0x68C0 + c;
        let brit = |b: u16| 0x6000 + (b << 4);

        // Colour first, then intensity: each must leave the other alone.
        let (vg, _, _) = run_color(&[cstat(5), brit(0xF), HALT]);
        assert_eq!(vg.color, 5, "the colour word set the colour");
        assert_eq!(vg.intensity, 0xF, "the intensity word set the intensity");

        // And in the other order, which is the case that would break if the
        // decode combined: a colour word carries C0 in its low byte, and read
        // as intensity that is a bright vector rather than no change.
        let (vg, _, _) = run_color(&[brit(0xA), cstat(3), HALT]);
        assert_eq!(vg.intensity, 0xA, "the colour word left intensity alone");
        assert_eq!(vg.color, 3);
    }

    #[test]
    fn the_two_fields_would_clobber_each_other_if_combined() {
        // The failure this decode exists to prevent, in both directions.
        // Under the combined rule each of the game's two words rewrites the
        // latch the other one owns.
        let words = [0x6000 + (0xA << 4), 0x68C0 + 5, HALT];

        let (vg, _, _) = run_color(&words);
        assert_eq!((vg.intensity, vg.color), (0xA, 5), "each word keeps to itself");

        let (vg, _, _) = run(&words);
        assert_eq!(
            vg.intensity, 6,
            "combined mode reads the colour word's C0 as an intensity"
        );

        // And the other way: an intensity word's F0 low byte becomes colour 0.
        let (vg, _, _) = run(&[0x68C0 + 5, 0x6000 + (0xF << 4), HALT]);
        assert_eq!(vg.color, 0, "combined mode reads the intensity as a colour");
    }

    #[test]
    fn the_selector_bit_is_not_mistaken_for_a_window_bit() {
        let (vg, _, _) = run_color(&[0x68C0 + 5, HALT]);
        assert_eq!(vg.window_bits, 0, "bit 11 stays out of the window mask");
    }

    #[test]
    fn combined_mode_is_unchanged_by_the_new_decode() {
        // Space Duel's `VGSTAT` always ORs 64 (VGUTR2.MAC:248-250), so bit 11
        // is clear in every status word it emits and both fields ride in one.
        let stat = 0x6400 + (4 << 4) + 3;
        let (vg, _, _) = run(&[stat, HALT]);
        assert_eq!(vg.intensity, 2, "bits 4-7 shifted one right, as before");
        assert_eq!(vg.color, 3);
    }

    #[test]
    fn scal_halves_by_binary_and_by_linear() {
        let (_, out, _) = run(&[scal(1, 0), vctr_short(8, 4, 7), HALT]);
        assert_eq!((out[0].x1, out[0].y1), (4, 2), "binary scale 1 is half size");

        let (_, out, _) = run(&[scal(0, 0x80), vctr_short(8, 4, 7), HALT]);
        assert_eq!((out[0].x1, out[0].y1), (4, 2), "linear 80 is half size");

        let (_, out, _) = run(&[scal(0, 0), vctr_short(8, 4, 7), HALT]);
        assert_eq!((out[0].x1, out[0].y1), (8, 4), "scale 0,0 is full size");
    }

    #[test]
    fn cntr_returns_the_beam_to_the_origin() {
        let (vg, _, _) = run(&[vctr_short(10, 12, 7), CNTR, HALT]);
        assert_eq!((vg.x, vg.y), (0, 0));
    }

    #[test]
    fn jsrl_and_rtsl_nest_and_return() {
        let mut m = mem(&[jsrl(0x100), vctr_short(2, 0, 7), HALT]);
        put(&mut m, 0x100, &[vctr_short(4, 0, 7), RTSL]);
        let mut vg = Vg::new();
        vg.reset(0);
        let mut out = Vec::new();
        assert_eq!(vg.run(&m, 10_000, &mut out), Stop::Halt);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].x1, 4, "the subroutine drew first");
        assert_eq!(out[1].x1, 6, "then execution resumed after the call");
        assert_eq!(vg.depth(), 0);
    }

    #[test]
    fn five_levels_nest_and_a_sixth_faults() {
        // Five chained calls, each one level deeper — the documented maximum.
        // Every level returns after its call, so the whole chain unwinds back
        // to the HALT rather than running on into blank memory.
        let mut m = mem(&[jsrl(0x100), HALT]);
        for level in 0..4u16 {
            put(
                &mut m,
                0x100 + level * 0x10,
                &[jsrl(0x110 + level * 0x10), RTSL],
            );
        }
        put(&mut m, 0x140, &[RTSL]);
        let mut vg = Vg::new();
        vg.reset(0);
        let mut out = Vec::new();
        assert_eq!(vg.run(&m, 10_000, &mut out), Stop::Halt);
        assert_eq!(vg.depth(), 0, "the chain unwound");

        // A sixth call from the deepest level overflows instead. Only the
        // innermost routine changes, so the difference is the extra level.
        put(&mut m, 0x140, &[jsrl(0x150), RTSL]);
        put(&mut m, 0x150, &[RTSL]);
        let mut vg = Vg::new();
        vg.reset(0);
        let mut out = Vec::new();
        assert_eq!(vg.run(&m, 10_000, &mut out), Stop::StackOverflow);
        assert!(!Stop::StackOverflow.is_clean());
    }

    #[test]
    fn rtsl_at_the_top_level_ends_the_list() {
        // How a shape entered by JSRL terminates when run standalone.
        let (_, out, stop) = run(&[vctr_short(2, 0, 7), RTSL]);
        assert_eq!(stop, Stop::Returned);
        assert!(stop.is_clean());
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn jmpl_transfers_without_pushing() {
        let mut m = mem(&[jmpl(0x080)]);
        put(&mut m, 0x080, &[vctr_short(6, 0, 7), HALT]);
        let mut vg = Vg::new();
        vg.reset(0);
        let mut out = Vec::new();
        assert_eq!(vg.run(&m, 10_000, &mut out), Stop::Halt);
        assert_eq!(vg.depth(), 0, "a jump does not nest");
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_runaway_list_trips_the_budget_instead_of_hanging() {
        let m = mem(&[jmpl(0)]);
        let mut vg = Vg::new();
        vg.reset(0);
        let mut out = Vec::new();
        let stop = vg.run(&m, 500, &mut out);
        assert_eq!(stop, Stop::Budget);
        assert!(!stop.is_clean());
        assert_eq!(vg.executed, 500);
    }

    #[test]
    fn execution_wraps_within_the_four_kiloword_space() {
        // The address field is twelve bits, so a list running off the end
        // wraps rather than reading outside the window.
        let mut m = mem(&[]);
        put(&mut m, 0xFFF, &[vctr_short(2, 0, 7)]);
        put(&mut m, 0x000, &[HALT]);
        let mut vg = Vg::new();
        vg.reset(0xFFF);
        let mut out = Vec::new();
        assert_eq!(vg.run(&m, 10, &mut out), Stop::Halt);
        assert_eq!(out.len(), 1);
    }

    /// Run a list with the beam watched, as `run` does without.
    fn run_traced(words: &[u16]) -> (Vg, Vec<Segment>, Vec<BeamMove>, Stop) {
        let m = mem(words);
        let mut vg = Vg::new();
        vg.reset(0);
        let (mut out, mut trace) = (Vec::new(), Vec::new());
        let stop = vg.run_trace(&m, 10_000, &mut out, &mut trace);
        (vg, out, trace, stop)
    }

    #[test]
    fn a_blanked_short_vector_moves_the_beam_and_draws_nothing() {
        let (vg, out, trace, stop) = run_traced(&[vctr_short(8, 4, 0), HALT]);
        assert_eq!(stop, Stop::Halt);
        assert!(out.is_empty(), "ZZ = 0 draws nothing");
        assert_eq!(trace.len(), 1, "but the beam still went somewhere");
        assert_eq!(
            trace[0],
            BeamMove {
                x0: 0,
                y0: 0,
                x1: 8,
                y1: 4,
                intensity: 0,
                color: 0,
                jump: false,
            }
        );
        assert_eq!((vg.x, vg.y), (8, 4));
    }

    #[test]
    fn a_blanked_long_vector_is_traced_too() {
        let long = vctr_long(-300, 700, 0);
        let (_, out, trace, stop) = run_traced(&[long[0], long[1], HALT]);
        assert_eq!(stop, Stop::Halt);
        assert!(out.is_empty());
        assert_eq!(trace.len(), 1);
        assert_eq!(trace[0].intensity, 0);
        assert_eq!((trace[0].x1, trace[0].y1), (-300, 700));
    }

    #[test]
    fn centring_is_a_jump_and_costs_no_time() {
        let (_, _, trace, stop) = run_traced(&[vctr_short(10, 6, 7), CNTR, HALT]);
        assert_eq!(stop, Stop::Halt);
        assert_eq!(trace.len(), 2);
        let jump = trace[1];
        assert!(jump.jump);
        assert_eq!((jump.x0, jump.y0), (10, 6));
        assert_eq!((jump.x1, jump.y1), (0, 0));
        assert_eq!(jump.ticks(), 0, "CNTR dumps the position, it does not sweep");
    }

    #[test]
    fn lit_moves_match_the_segments_exactly() {
        let words = [
            color(4, 6),
            vctr_short(8, 4, 1),
            vctr_short(6, 0, 0),
            vctr_short(-2, -10, 3),
            CNTR,
            vctr_short(4, 4, 7),
            HALT,
        ];
        let (_, out, trace, stop) = run_traced(&words);
        assert_eq!(stop, Stop::Halt);
        assert_eq!(out.len(), 3, "one blanked move of the four vectors");
        assert_eq!(trace.len(), 5, "four vectors and the centre");

        let lit: Vec<&BeamMove> = trace.iter().filter(|m| m.intensity != 0).collect();
        assert_eq!(lit.len(), out.len());
        for (m, s) in lit.iter().zip(&out) {
            assert_eq!(
                (m.x0, m.y0, m.x1, m.y1, m.intensity, m.color),
                (s.x0, s.y0, s.x1, s.y1, s.intensity, s.color)
            );
            assert!(!m.jump);
        }
    }

    #[test]
    fn scaling_applies_to_traced_moves() {
        // SCAL 1,0 halves, and the blanked move is scaled the same as a lit
        // one — the beam does not know whether it is on.
        let (_, out, trace, stop) =
            run_traced(&[scal(1, 0), vctr_short(8, 4, 0), vctr_short(8, 4, 7), HALT]);
        assert_eq!(stop, Stop::Halt);
        assert_eq!(trace.len(), 2);
        assert_eq!((trace[0].x1, trace[0].y1), (4, 2));
        assert_eq!((trace[1].x1, trace[1].y1), (8, 4));
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].x1, out[0].y1), (8, 4));
    }

    #[test]
    fn ticks_are_the_chebyshev_length() {
        let move_of = |x1, y1| BeamMove {
            x0: 0,
            y0: 0,
            x1,
            y1,
            intensity: 7,
            color: 0,
            jump: false,
        };
        assert_eq!(move_of(30, 8).ticks(), 30);
        assert_eq!(move_of(8, -30).ticks(), 30);
        assert_eq!(move_of(-12, 12).ticks(), 12);
        assert_eq!(move_of(0, 0).ticks(), 0);
    }

    #[test]
    fn intensity_drive_is_zero_off_full_at_seven_and_rises_between() {
        assert_eq!(intensity_drive(0), 0.0, "code 0 is off");
        assert_eq!(intensity_drive(7), 1.0, "code 7 is nominal full drive");
        for code in 1..7u8 {
            assert!(
                intensity_drive(code) < intensity_drive(code + 1),
                "code {code} must be dimmer than {}",
                code + 1
            );
        }
        // The field is three bits; anything above is not a brightness.
        assert_eq!(intensity_drive(8), 0.0);
        assert_eq!(intensity_drive(0x0F), 1.0);
    }

    #[test]
    fn the_colour_boards_two_intensity_routes_meet_on_one_scale() {
        assert_eq!(intensity_drive_color(0), 0.0, "code 0 is off");
        assert_eq!(intensity_drive_color(0x0F), 1.0, "a full nibble is full");
        // The pair the running game emits: one VGBRIT of C0, seen through a
        // status word and through a vector's ZZ field.
        assert_eq!(
            intensity_drive_color(6),
            intensity_drive_color(12),
            "ZZ 6 and nibble C are the same brightness"
        );
        // Monotonic within each route, but not across the two: a ZZ code of 7
        // is `VGBRIT` `E0` and outshines a status nibble of 8, which is `80`.
        for code in 1..7u8 {
            assert!(intensity_drive_color(code) < intensity_drive_color(code + 1));
        }
        for code in 8..0x0Fu8 {
            assert!(intensity_drive_color(code) < intensity_drive_color(code + 1));
        }
        assert!(intensity_drive_color(7) > intensity_drive_color(8));
    }

    #[test]
    fn the_diagnostic_corner_vector_lands_on_full_scale() {
        // XYSIG.MAC:143-145, the XY board's own diagnostic, driving the beam
        // from the centre to a corner. If our decode of SCAL and of a long
        // vector is right, this lands exactly on the full-scale corner — which
        // is where FULL_SCALE_X and FULL_SCALE_Y come from.
        let long = vctr_long(512, 384, 1);
        let (vg, out, stop) = run(&[CNTR, scal(1, 0), long[0], long[1], HALT]);
        assert_eq!(stop, Stop::Halt);
        assert_eq!((vg.x, vg.y), (FULL_SCALE_X, FULL_SCALE_Y));
        // Its ZZ is 1, which defers to the latch, and the latch is 0 from
        // reset — the diagnostic sets no status word first. So this sequence
        // moves the beam without lighting it, which is a fact about the
        // diagnostic and not about the scale.
        assert!(out.is_empty());

        // And back again: TEST2 returns to the centre the same way.
        let back = vctr_long(-512, -384, 1);
        let (vg, _, stop) = run(&[
            CNTR,
            scal(1, 0),
            long[0],
            long[1],
            CNTR,
            back[0],
            back[1],
            HALT,
        ]);
        assert_eq!(stop, Stop::Halt);
        assert_eq!((vg.x, vg.y), (-FULL_SCALE_X, -FULL_SCALE_Y));
    }

    #[test]
    fn tracing_does_not_change_what_is_drawn() {
        let words = [
            color(2, 5),
            vctr_short(8, 4, 1),
            vctr_short(6, 0, 0),
            CNTR,
            scal(1, 0),
            vctr_short(-2, -10, 3),
            HALT,
        ];
        let m = mem(&words);

        let mut plain = Vg::new();
        plain.reset(0);
        let mut plain_out = Vec::new();
        let plain_stop = plain.run(&m, 10_000, &mut plain_out);

        let (traced, traced_out, _, traced_stop) = run_traced(&words);
        assert_eq!(plain_stop, traced_stop);
        assert_eq!(plain_out, traced_out);
        assert_eq!((plain.x, plain.y), (traced.x, traced.y));
        assert_eq!(plain.executed, traced.executed);
    }
}
