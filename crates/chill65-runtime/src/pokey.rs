//! POKEY ×2 — registers, the RANDOM generator, and the audio polynomials.
//!
//! # Scope
//!
//! Audio output was out of scope for Phase 2 (plan §5 calls POKEY "not the
//! interesting part", and the gate is headless). It is synthesised now: the
//! free-running parts are [`Poly4`], [`Poly5`] and the two clock divisors, and
//! [`Audio`] is the per-channel path that samples them — dividers, distortion
//! select, high-pass, volume — summing to the chip's six-bit `snd`.
//!
//! Nothing yet *emits* the stream; [`Pokey::snd`] gives the level at whichever
//! cycle the chip was last advanced to, and carrying that out of `run_frame` is
//! the next task.
//!
//! # Where the audio path comes from
//!
//! **Measured against the verilated core, not transcribed from it.** The core's
//! `rtl/Pokey/` is © 2013 Mark Watson and licensed for non-commercial use only,
//! with the notice extending to derived works, so it cannot be a source for an
//! MIT repository. Everything in the audio path below was instead recovered by
//! running an original fixture on the core and reading `SOUT` back — observing
//! behaviour rather than copying expression. Each item records the sequence or
//! period it was identified from, so the evidence is checkable and the tests
//! assert the measurements rather than the implementation.
//!
//! Two things this does *not* cover, both recorded where they bite rather than
//! only here: poly17's audio tap, which no fixture has isolated, and the joined
//! 16-bit and high-pass modes, which the game is not known to use and which no
//! fixture has exercised. Each carries a comment saying so.
//!
//! # No delayed taps here
//!
//! POKEY implementations commonly carry the polynomial bits through a delay line
//! so the sampling instant lines up with the rest of the chip. No such delay is
//! modelled below, and that is a measurement result rather than an omission:
//! plain undelayed shift registers reproduce both captured sequences exactly,
//! term for term, over every period observed. A delay in the real part would be
//! a constant phase offset in the output, which is precisely what aligning our
//! stream against the core's establishes — so if one exists it will appear there
//! as an offset, in a place designed to measure it, rather than being guessed at
//! here.
//!
//! What was never out of scope is `RANDOM`, because the game reads it to make
//! **gameplay** decisions, not just noise:
//!
//! - `CATOUT.MAC:22` — `LDA RANDOM / AND #0E0 / ADC #010 / STA XB` picks the
//!   cat's X position on the bitmap. A constant here would spawn every cat in
//!   the same column.
//! - `CCUBE.MAC` reads it seven times.
//!
//! So the LFSR below is the real one, transcribed from the core's own POKEY
//! (`rtl/Pokey/pokey_poly_17_9.v`), not a stand-in.
//!
//! # Addressing
//!
//! `AddresDecoders.v:59` puts both chips in `9800-9BFF`, and `AudioOutput.v:16`
//! splits them on `BA[9]`:
//!
//! ```verilog
//! wire cs_pokey3B = ~CIOn & ~BA[9];   // 9800-99FF — the game's POKEY0
//! wire cs_pokey3D = ~CIOn &  BA[9];   // 9A00-9BFF — the game's POKEY1
//! ```
//!
//! Each chip sees only `BA[3:0]` (`AudioOutput.v:28`), so its sixteen registers
//! mirror every 16 bytes through the 512-byte window. The game's equates agree:
//! `POKEY0 = 9800`, `POKEY1 = 9A00`, `RANDOM = 0A+POKEY0` (`CG.MAC:34-40`).
//!
//! # Clocking
//!
//! `AudioOutput.v:26` gives the chips `.ce(ce2Hd)` — the delayed 1.25 MHz CPU
//! enable — and `SupportChips.v:240` ties `.enable_179(1'b1)`. The poly
//! therefore **advances exactly once per CPU cycle**, with no further division.
//!
//! It is advanced lazily, on access, rather than on every tick: the result is a
//! pure function of the elapsed cycle count, so catching up at the moment of
//! observation is exactly equivalent and costs nothing when nobody is looking.
//!
//! # Init mode
//!
//! `pokey.v:709`: `initmode = ~(skctl_next[1] | skctl_next[0])`. While both low
//! bits of SKCTL are clear the poly is held in init and bit 16 is forced to
//! zero. This is not a corner case here — it is the state from reset until
//! `CST.MAC:18-19` writes `SKCTL = 7` ("FAST POT") to both chips, having first
//! zeroed all sixteen registers.

/// The 17-bit polynomial counter that drives `RANDOM`.
///
/// Transcribed literally from `pokey_poly_17_9.v:47-70` so that the sequence
/// agrees with the reference core bit for bit, rather than from one of the
/// several mutually inconsistent prose descriptions of POKEY's poly17.
///
/// **That file is one of Mark Watson's**, so this transcription predates — and
/// contradicts — the sourcing rule stated in the module docs. It is left alone
/// here rather than quietly rewritten, because it is a decision to take rather
/// than a bug to fix. It can be resolved without changing a line of it:
/// distortion `AUDC = 0x8` puts poly17 on the output the same way `0xC` puts
/// poly4 there, so the sequence can be measured off the core and this
/// implementation either confirmed by it or replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Poly17 {
    /// `shift_reg` — 17 bits.
    pub shift: u32,
    /// `select_9_17_del_reg` — the mode selector, delayed one step.
    pub sel_delayed: bool,
}

impl Default for Poly17 {
    fn default() -> Self {
        Self::new()
    }
}

impl Poly17 {
    /// `shift_reg <= 17'b01010101010101010` on reset (`pokey_poly_17_9.v:39`).
    pub const RESET: u32 = 0x0_AAAA;

    pub fn new() -> Self {
        Poly17 {
            shift: Self::RESET,
            sel_delayed: false,
        }
    }

    /// One step. `select9` is `AUDCTL` bit 7; `init` is the SKCTL hold.
    pub fn step(&mut self, select9: bool, init: bool) {
        let s = self.shift;
        // feedback = shift_reg[13] ~^ shift_reg[8]  — XNOR, not XOR.
        let feedback = !((s >> 13) ^ (s >> 8)) & 1;

        let mut next = ((s >> 9) & 0xFF) << 8; // [15:8] <= [16:9]
        next |= feedback << 7; // [7]    <= feedback
        next |= (s >> 1) & 0x7F; // [6:0]  <= [7:1]

        let sel = select9 as u32;
        let bit16 = ((feedback & self.sel_delayed as u32) | ((s & 1) & (sel ^ 1))) & !(init as u32);
        next |= (bit16 & 1) << 16;

        self.shift = next & 0x1_FFFF;
        self.sel_delayed = select9;
    }

    /// `rand_out = ~shift_reg[15:8]` (`pokey_poly_17_9.v:73`).
    pub fn random(&self) -> u8 {
        !((self.shift >> 8) as u8)
    }
}

/// The audio clock divisors, **measured**, not taken from a datasheet.
///
/// A pure tone with `AUDF = 0` toggles its output once per divider tick, so its
/// period is two ticks and the divisor is half the measured period. Captured
/// from the verilated core's `SOUT`, one sample per CPU cycle:
///
/// ```text
/// AUDCTL 00   pure-tone period 56 cycles   -> divisor 28
/// AUDCTL 01   pure-tone period 228 cycles  -> divisor 114
/// ```
///
/// `AUDCTL` bit 0 selects the slow clock. The ratio 114/28 = 4.07 is the
/// documented 63.9 kHz : 15.7 kHz relationship, which is the cross-check that
/// these two numbers are the two clocks and not something else.
///
/// On an Atari home computer these divide 1.79 MHz. This board clocks POKEY at
/// the 1.25 MHz CPU rate instead — `AudioOutput.v:26` gives the chips `ce2Hd`
/// and `SupportChips.v:240` ties `.enable_179(1'b1)` — so the *divisors* are the
/// chip's but the resulting frequencies are 44.6 kHz and 11.0 kHz.
pub const DIV_FAST: u32 = 28;
/// The slow audio clock divisor; see [`DIV_FAST`].
pub const DIV_SLOW: u32 = 114;

/// The 4-bit polynomial counter — `x^4 + x^3 + 1`.
///
/// **Measured, not transcribed.** `rtl/Pokey/` is (c) 2013 Mark Watson and
/// licensed non-commercially, so nothing here comes from it; see the README.
/// Instead an original fixture was run on the verilated core and `SOUT` read
/// back. Recovering the register took three steps, because none of them can be
/// read off directly.
///
/// **It free-runs.** Distortion `AUDC = 0xC` selects poly4 with poly5 out of
/// circuit, and the channel samples the polynomial each time its divider
/// underflows — so what reaches the output is a *decimation*, not the sequence.
/// Changing `AUDF` changes which terms are sampled, and it does: `AUDF` 0, 1 and
/// 2 give three different 15-term readings, the second being the first taken
/// every 2nd term and the third every 3rd, exactly as a free-running register
/// sampled every `28 * (AUDF + 1)` cycles must.
///
/// **It runs at the CPU clock, not the base clock.** `AUDCTL` bit 6 puts this
/// channel on the main clock, where its divider is `AUDF + 4` *cycles*. The
/// output period then measures 60 cycles at `AUDF = 0` and 120 at `AUDF = 4` —
/// 15 underflows either way. Fifteen distinct values inside 60 cycles is
/// impossible for a register stepping once per 28-cycle base tick, so it steps
/// with the CPU.
///
/// **Two decimations then solve it.** The slow path samples every 28 cycles and
/// the fast path every 4; 28 and 4 are each invertible modulo 15, so each
/// reading can be un-decimated into a candidate register sequence. They agree,
/// up to rotation:
///
/// ```text
/// via /28   1,1,0,1,0,1,1,1,1,0,0,0,1,0,0
/// via /4    same sequence, rotated by 4
/// ```
///
/// That agreement is the check — two unrelated sampling rates cannot produce
/// consistent nonsense. A search over every tap subset, both polarities and
/// every start state leaves one machine: taps 0 and 1, XOR. It is
/// `x^4 + x^3 + 1`, the standard maximal 4-bit polynomial, which is independent
/// agreement with the published descriptions of the chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Poly4 {
    /// 4 bits; the output is bit 0.
    pub shift: u8,
}

impl Default for Poly4 {
    fn default() -> Self {
        Self::new()
    }
}

impl Poly4 {
    /// A state on the measured cycle, not the state at reset.
    ///
    /// The fixture idles for tens of thousands of cycles before the measurement
    /// window, so the phase within the sequence was observed and its origin was
    /// not. Every state is on the one cycle, so this is the right sequence in an
    /// arbitrary phase; aligning to the core's absolute phase is what task #62
    /// does.
    pub const RESET: u8 = 0b1011;

    pub fn new() -> Self {
        Poly4 { shift: Self::RESET }
    }

    /// One CPU cycle.
    pub fn step(&mut self) {
        let fb = (self.shift ^ (self.shift >> 1)) & 1;
        self.shift = ((self.shift >> 1) | (fb << 3)) & 0x0F;
    }

    pub fn bit(&self) -> bool {
        self.shift & 1 != 0
    }
}

/// The 5-bit polynomial counter — `x^5 + x^3 + 1`.
///
/// **Measured, not transcribed**, the same way as [`Poly4`] and with one extra
/// obstacle: poly5 never reaches the output at all.
///
/// Sweeping all eight distortion settings showed `0x2` and `0x6` producing
/// identical output, and `0xA` and `0xE` likewise. That is the documented shape
/// of `AUDC` bits 7:5 confirmed by measurement: **bit 7 selects whether poly5 is
/// in the chain**, bits 6:5 pick poly17, poly4 or a pure tone after it. poly5
/// only ever *gates* the stage behind it:
///
/// ```text
/// AUDC  period (divider ticks)
///  0x0  none          poly5 + poly17
///  0x2  62            poly5 + pure tone
///  0x4  465 = 15x31   poly5 + poly4
///  0x6  62            poly5 + pure tone, same as 0x2
///  0x8  none          poly17
///  0xA  2             pure tone
///  0xC  15            poly4
///  0xE  2             pure tone, same as 0xA
/// ```
///
/// 62 rather than 31 because the gated stage *toggles*: an odd number of gate
/// events per polynomial period doubles it. So poly5's samples are the output's
/// **transitions**, and from there the same three steps as poly4 apply —
/// free-running, at the CPU clock, solved from the `/28` and `/4` decimations,
/// which agree up to rotation. One machine survives: taps 0 and 2, XNOR. It is
/// `x^5 + x^3 + 1`, the standard maximal 5-bit polynomial.
///
/// The polarity has a second, independent witness. XNOR excludes the all-ones
/// state where XOR excludes all-zeros, so an XNOR register emits 15 ones per 31
/// terms and an XOR register would emit 16. Fifteen is odd, and an odd number of
/// gate events per period is exactly what makes the output period 62 rather than
/// 31 — the polarity and the measured period imply each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Poly5 {
    /// 5 bits; the output is bit 0.
    pub shift: u8,
}

impl Default for Poly5 {
    fn default() -> Self {
        Self::new()
    }
}

impl Poly5 {
    /// A state on the measured cycle; see [`Poly4::RESET`] for why this is a
    /// phase and not an origin.
    pub const RESET: u8 = 0b01111;

    pub fn new() -> Self {
        Poly5 { shift: Self::RESET }
    }

    /// One CPU cycle.
    pub fn step(&mut self) {
        let fb = !(self.shift ^ (self.shift >> 2)) & 1;
        self.shift = ((self.shift >> 1) | (fb << 4)) & 0x1F;
    }

    pub fn bit(&self) -> bool {
        self.shift & 1 != 0
    }
}

/// One chip's four-channel audio path.
///
/// # Where this comes from
///
/// The register semantics are the chip's published behaviour (Atari's *Hardware
/// Manual*, De Re Atari and the Altirra Hardware Reference Manual all describe
/// the same divider/distortion/volume arrangement); nothing is taken from
/// `rtl/Pokey/`. Where a number could be measured on this board it was, and the
/// comment says so. Where it could not, the comment says that instead — task
/// #62 compares this whole stream against the core and is where the unmeasured
/// parts get settled.
///
/// # Not modelled
///
/// The serial port and the two-tone mode are absent. This board's driver writes
/// `SKCTL = 7` (`CST.MAC:18-19`) and never enables either, so their absence
/// cannot change anything the game does. Nothing reads back a serial or two-tone
/// result, so there is no silent-wrong-answer path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Audio {
    /// Prescaler for the base clock, counting CPU cycles.
    base: u32,
    /// Per-channel countdown, in that channel's own clock ticks.
    counter: [u16; 4],
    /// Per-channel output flip-flop.
    out: [bool; 4],
    /// High-pass flip-flops for channels 0 and 1.
    hp: [bool; 2],
    pub poly4: Poly4,
    pub poly5: Poly5,
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl Audio {
    pub fn new() -> Self {
        Audio {
            base: 0,
            counter: [0; 4],
            out: [false; 4],
            hp: [false; 2],
            poly4: Poly4::new(),
            poly5: Poly5::new(),
        }
    }

    /// The divider reload for an 8-bit channel.
    ///
    /// **Measured.** A channel underflows every `AUDF + 1` base-clock ticks, and
    /// every `AUDF + 4` CPU cycles on the main clock. Both were read off the
    /// core: a pure tone at `AUDF = 0` has a 56-cycle period, two ticks of the
    /// 28-cycle base clock; and on the main clock the poly4 output period is 60
    /// cycles at `AUDF = 0` and 120 at `AUDF = 4`, fifteen underflows either way,
    /// so the divider is 4 and 8 cycles. A counter reloaded with `n` fires every
    /// `n + 1` ticks, hence the extra 3 on the main clock.
    fn reload8(regs: &[u8; 16], ch: usize, fast: bool) -> u16 {
        u16::from(regs[ch * 2]) + if fast { 3 } else { 0 }
    }

    /// The divider reload for a joined 16-bit pair, low channel's `AUDF` as the
    /// low byte.
    ///
    /// **Not measured.** The game is not known to join channels, so no fixture
    /// exercised this; the `+ 6` on the main clock is the documented figure
    /// rather than an observed one. If task #62 finds a discrepancy in a joined
    /// mode, this is the first line to doubt.
    fn reload16(regs: &[u8; 16], lo: usize, fast: bool) -> u16 {
        let v = u16::from(regs[lo * 2]) | (u16::from(regs[(lo + 1) * 2]) << 8);
        v.wrapping_add(if fast { 6 } else { 0 })
    }

    /// Count one tick of this channel's clock; true on underflow.
    fn tick(&mut self, ch: usize, reload: u16) -> bool {
        if self.counter[ch] == 0 {
            self.counter[ch] = reload;
            true
        } else {
            self.counter[ch] -= 1;
            false
        }
    }

    /// Update a channel's output flip-flop, the channel having just underflowed.
    ///
    /// `AUDC` bits 7:5 select the distortion. **The whole table was measured**;
    /// see [`Poly5`] for the eight periods that produced it. Bit 7 says whether
    /// poly5 is in circuit, and bits 6:5 choose what follows it:
    ///
    /// ```text
    /// 00  poly17 (or poly9)   01  pure tone
    /// 10  poly4               11  pure tone
    /// ```
    ///
    /// With poly5 in circuit it *gates*: when its bit is clear the flip-flop
    /// holds its value rather than updating, which is why those settings measure
    /// twice the period. The pure-tone case toggles; the polynomial cases take
    /// the polynomial's bit, which is why poly4 alone measures period 15 rather
    /// than 30.
    fn update(&mut self, ch: usize, regs: &[u8; 16], poly17_bit: bool) {
        let audc = regs[ch * 2 + 1];
        // Bit 7 clear puts poly5 in circuit, where it gates the update.
        if audc & 0x80 == 0 && !self.poly5.bit() {
            return;
        }
        self.out[ch] = match (audc >> 5) & 0x03 {
            0b00 => poly17_bit,
            0b10 => self.poly4.bit(),
            // 0b01 and 0b11 are both the pure tone, measured identical.
            _ => !self.out[ch],
        };
    }

    /// One CPU cycle of the whole audio path.
    ///
    /// `poly17_bit` is the 17/9-bit polynomial's audio tap. **Which bit of that
    /// register the audio path uses is not established** — [`Poly17`] models it
    /// for `RANDOM`, which reads a different field entirely, and no fixture has
    /// yet isolated the audio tap. Distortion `AUDC = 0x8` would measure it the
    /// same way `0xC` measured poly4. Until then this is the one input to the
    /// pipeline that is a guess, and task #62 will show it as a mismatch on any
    /// channel using a poly17 distortion.
    pub fn step(&mut self, regs: &[u8; 16], poly17_bit: bool) {
        // Both small polynomials free-run at the CPU clock, independently of
        // every divider; see the note on `Poly4` for the measurement that
        // established the rate.
        self.poly4.step();
        self.poly5.step();

        let audctl = regs[reg::AUDCTL];

        // AUDCTL bit 0 selects the slow base clock for every channel that is not
        // individually on the main clock. Both divisors are measured; see
        // `DIV_FAST`.
        let div = if audctl & 0x01 != 0 { DIV_SLOW } else { DIV_FAST };
        self.base += 1;
        let base = self.base >= div;
        if base {
            self.base = 0;
        }

        // AUDCTL bit 6 puts channel 1 on the main clock, bit 5 channel 3. Bit 4
        // joins channels 1+2 into one 16-bit divider and bit 3 joins 3+4.
        let fast = [audctl & 0x40 != 0, false, audctl & 0x20 != 0, false];
        let join = [audctl & 0x10 != 0, audctl & 0x08 != 0];

        let mut fired = [false; 4];
        for pair in 0..2 {
            let lo = pair * 2;
            if join[pair] {
                // One divider, clocked by the low channel's clock, driving the
                // high channel's output. Software silences the low channel.
                if (fast[lo] || base) && self.tick(lo, Self::reload16(regs, lo, fast[lo])) {
                    fired[lo + 1] = true;
                }
            } else {
                for ch in [lo, lo + 1] {
                    if (fast[ch] || base) && self.tick(ch, Self::reload8(regs, ch, fast[ch])) {
                        fired[ch] = true;
                    }
                }
            }
        }

        for ch in 0..4 {
            if fired[ch] {
                self.update(ch, regs, poly17_bit);
            }
        }

        // AUDCTL bit 2 high-passes channel 1 against channel 3, bit 1 channel 2
        // against channel 4: a flip-flop samples the filtered channel's output
        // at each of the clocking channel's underflows, and the two are then
        // XORed. **Not measured** — no fixture has exercised it.
        if audctl & 0x04 != 0 && fired[2] {
            self.hp[0] = self.out[0];
        }
        if audctl & 0x02 != 0 && fired[3] {
            self.hp[1] = self.out[1];
        }
    }

    /// A channel's output after the high-pass, if it is enabled for it.
    fn channel_out(&self, ch: usize, regs: &[u8; 16]) -> bool {
        let audctl = regs[reg::AUDCTL];
        match ch {
            0 if audctl & 0x04 != 0 => self.out[0] ^ self.hp[0],
            1 if audctl & 0x02 != 0 => self.out[1] ^ self.hp[1],
            _ => self.out[ch],
        }
    }

    /// One channel's contribution, 0-15.
    ///
    /// `AUDC` bits 3:0 are the level and bit 4 is volume-only: with it set the
    /// level is emitted continuously and the flip-flop is ignored, which is how
    /// software plays sampled sound. Otherwise the flip-flop gates the level.
    pub fn level(&self, ch: usize, regs: &[u8; 16]) -> u8 {
        let audc = regs[ch * 2 + 1];
        let vol = audc & 0x0F;
        if audc & 0x10 != 0 || self.channel_out(ch, regs) {
            vol
        } else {
            0
        }
    }

    /// The chip's `snd`: four channels of 0-15 summed, so 0-60 in six bits.
    pub fn snd(&self, regs: &[u8; 16]) -> u8 {
        (0..4).map(|ch| self.level(ch, regs)).sum()
    }

    /// `STIMER`: restart the whole divider chain and clear the output
    /// flip-flops, so the channels resume from a known phase together. This is
    /// the documented purpose of the register — software writes it after setting
    /// frequencies to stop the channels beating against each other.
    ///
    /// **The base-clock prescaler restarts too**, which is measured rather than
    /// assumed. The fixture in `chill65-diff` strobes STIMER on one chip and
    /// then the other, four cycles apart — one `STA` — and the core's output
    /// puts the two chips' first edges exactly those four cycles apart. If the
    /// prescaler ran free the two chips would share the 28-cycle grid and both
    /// would fire on the same tick regardless of when they were strobed, which
    /// is not what the hardware does.
    pub fn stimer(&mut self, regs: &[u8; 16]) {
        self.base = 0;
        let audctl = regs[reg::AUDCTL];
        let fast = [audctl & 0x40 != 0, false, audctl & 0x20 != 0, false];
        let join = [audctl & 0x10 != 0, audctl & 0x08 != 0];
        for pair in 0..2 {
            let lo = pair * 2;
            if join[pair] {
                self.counter[lo] = Self::reload16(regs, lo, fast[lo]);
                self.counter[lo + 1] = 0;
            } else {
                for ch in [lo, lo + 1] {
                    self.counter[ch] = Self::reload8(regs, ch, fast[ch]);
                }
            }
        }
        // **Set, not cleared** — measured. With the flip-flops cleared here,
        // every tone in the `chill65-diff` fixture comes out inverted against
        // the core: identical periods, identical run lengths, opposite phase by
        // exactly half a period each. Setting them is the one change that fixes
        // that, and it cannot instead be a global output inversion — the poly4
        // sequence was identified from the core's own output and would break.
        self.out = [true; 4];
        self.hp = [false; 2];
    }
}

/// Register indices within a chip.
pub mod reg {
    pub const AUDCTL: usize = 0x08;
    /// Write-only, sharing an address with `KBCODE`'s read.
    pub const STIMER: usize = 0x09;
    pub const ALLPOT: usize = 0x08;
    pub const KBCODE: usize = 0x09;
    pub const RANDOM: usize = 0x0A;
    pub const SERIN: usize = 0x0D;
    pub const IRQST: usize = 0x0E;
    pub const SKCTL: usize = 0x0F;
    pub const SKSTAT: usize = 0x0F;
}

pub struct Pokey {
    /// Last value written to each register. Audio registers are stored and
    /// never acted on; see the module docs.
    pub regs: [u8; 16],
    pub poly: Poly17,
    pub audio: Audio,
    /// While set, [`Pokey::advance_to`] appends this chip's `snd` for every
    /// cycle it steps through. Off by default: with nobody draining it the
    /// buffer would grow without bound, and every existing caller runs frames
    /// forever without asking for audio.
    pub record_audio: bool,
    /// One `snd` per CPU cycle since the buffer was last cleared. The sample
    /// for a cycle is the level *after* that cycle's step, so index `i` is the
    /// level during cycle `advanced_to - len + i + 1`.
    pub samples: Vec<u8>,
    /// Machine cycle the poly has been advanced to.
    pub advanced_to: u64,
}

impl Default for Pokey {
    fn default() -> Self {
        Self::new()
    }
}

impl Pokey {
    pub fn new() -> Self {
        Pokey {
            regs: [0; 16],
            poly: Poly17::new(),
            audio: Audio::new(),
            record_audio: false,
            samples: Vec::new(),
            advanced_to: 0,
        }
    }

    /// True while the poly is held in reset — SKCTL bits 1:0 both clear.
    #[inline]
    pub fn init_mode(&self) -> bool {
        self.regs[reg::SKCTL] & 0x03 == 0
    }

    /// AUDCTL bit 7 selects the 9-bit polynomial.
    #[inline]
    pub fn select9(&self) -> bool {
        self.regs[reg::AUDCTL] & 0x80 != 0
    }

    /// Bring the poly and the audio path up to `cycle`, one step per CPU cycle.
    ///
    /// The poly17 bit handed to the audio path is the one standing *before* that
    /// cycle's step, so the audio path sees the value the register held during
    /// the cycle rather than the one it is about to take. Nothing has verified
    /// that ordering; it is a one-cycle question and task #62's alignment is
    /// where a constant offset like it would show up.
    pub fn advance_to(&mut self, cycle: u64) {
        let select9 = self.select9();
        let init = self.init_mode();
        for _ in self.advanced_to..cycle {
            let bit = self.poly.shift & 1 != 0;
            self.poly.step(select9, init);
            self.audio.step(&self.regs, bit);
            if self.record_audio {
                self.samples.push(self.audio.snd(&self.regs));
            }
        }
        self.advanced_to = self.advanced_to.max(cycle);
    }

    /// This chip's six-bit `snd`, as of the last cycle it was advanced to.
    pub fn snd(&self) -> u8 {
        self.audio.snd(&self.regs)
    }

    /// Read a register. `index` is `BA[3:0]`.
    pub fn read(&mut self, index: usize, cycle: u64) -> u8 {
        self.advance_to(cycle);
        match index & 0x0F {
            reg::RANDOM => self.poly.random(),
            // Pots idle. This board reads a trackball, not paddles; ALLPOT
            // reads zero, meaning "no pot still counting", so anything that
            // polled it for completion would proceed rather than hang.
            0x00..=0x08 => 0,
            reg::KBCODE => 0,
            reg::SERIN => 0,
            // IRQST is active-low: all ones is "nothing pending". POKEY's IRQ
            // is not wired to this CPU anyway — interrupts come from the video
            // counters (see `frame.rs`).
            reg::IRQST => 0xFF,
            // SKSTAT idle.
            reg::SKSTAT => 0xFF,
            _ => 0xFF,
        }
    }

    /// Write a register. `index` is `BA[3:0]`.
    ///
    /// The poly is caught up *before* the write lands, so a change to SKCTL or
    /// AUDCTL takes effect from that cycle onward and not retroactively.
    pub fn write(&mut self, index: usize, value: u8, cycle: u64) {
        self.advance_to(cycle);
        let index = index & 0x0F;
        self.regs[index] = value;
        // STIMER acts on the registers as they stand after the write, so the
        // dividers reload from the frequencies software has just finished
        // setting. Its value is immaterial; the write itself is the strobe.
        if index == reg::STIMER {
            self.audio.stimer(&self.regs);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Free-running: SKCTL enabled, 17-bit mode.
    fn running() -> Pokey {
        let mut p = Pokey::new();
        p.write(reg::SKCTL, 0x07, 0); // what CST.MAC:18 writes
        p
    }

    #[test]
    fn the_reset_state_matches_the_rtl() {
        let p = Poly17::new();
        assert_eq!(p.shift, 0b0_1010_1010_1010_1010);
        assert_eq!(p.shift, 0xAAAA);
    }

    #[test]
    fn feedback_is_xnor_not_xor() {
        // Getting this backwards yields a plausible-looking but wrong sequence.
        // The reset word makes it discriminating: bit 13 is set and bit 8 is
        // clear, so XOR would feed 1 into bit 7 and XNOR feeds 0.
        let mut p = Poly17::new();
        assert_eq!((p.shift >> 13) & 1, 1);
        assert_eq!((p.shift >> 8) & 1, 0);
        p.step(false, false);
        assert_eq!((p.shift >> 7) & 1, 0, "XNOR(1,0) = 0; XOR would give 1");
    }

    #[test]
    fn the_seventeen_bit_sequence_has_the_full_period() {
        // A 17-stage maximal LFSR runs 2^17 - 1 = 131071 states. Measuring it
        // is the strongest available check that the taps are right: a wrong tap
        // almost always shortens the cycle.
        let start = Poly17::new();
        let mut p = start;
        let mut steps = 0u32;
        loop {
            p.step(false, false);
            steps += 1;
            if p == start {
                break;
            }
            assert!(steps < 200_000, "no cycle found within 200k steps");
        }
        assert_eq!(steps, 131_071, "2^17 - 1");
    }

    #[test]
    fn the_nine_bit_mode_has_a_shorter_period() {
        // AUDCTL bit 7 taps the polynomial out early. Measured rather than
        // asserted from memory.
        //
        // The settling matters and is not arbitrary. Bits [16:8] form the
        // self-contained 9-bit LFSR, but bits [7:0] are a *delayed copy of the
        // feedback stream* — they need eight steps to become consistent with
        // it. Sample the state earlier than that and the 17-bit word never
        // returns to its starting value at all, so the period looks unbounded.
        let mut p = Poly17::new();
        for _ in 0..16 {
            p.step(true, false);
        }
        let start = p;
        let mut steps = 0u32;
        loop {
            p.step(true, false);
            steps += 1;
            if p == start {
                break;
            }
            assert!(steps < 200_000, "no cycle found");
        }
        assert_eq!(steps, 511, "2^9 - 1");
        assert!(steps < 131_071, "shorter than the 17-bit sequence");
    }

    #[test]
    fn random_advances_with_elapsed_machine_cycles() {
        let mut p = running();
        let a = p.read(reg::RANDOM, 100);
        let b = p.read(reg::RANDOM, 200);
        assert_ne!(a, b, "100 cycles of elapsed time must move it");

        // Catching up lazily must equal stepping the whole way.
        let mut lazy = running();
        let mut eager = running();
        for c in 1..=5000 {
            eager.advance_to(c);
        }
        lazy.advance_to(5000);
        assert_eq!(lazy.poly, eager.poly, "lazy catch-up is exact");
        assert_eq!(lazy.read(reg::RANDOM, 5000), eager.read(reg::RANDOM, 5000));
    }

    #[test]
    fn reading_random_twice_in_the_same_cycle_gives_the_same_value() {
        let mut p = running();
        let a = p.read(reg::RANDOM, 1234);
        let b = p.read(reg::RANDOM, 1234);
        assert_eq!(a, b, "the poly is a function of the cycle count");
    }

    #[test]
    fn random_is_held_in_init_until_skctl_is_written() {
        // From reset SKCTL is 0, so initmode is high and bit 16 is forced low.
        let mut held = Pokey::new();
        assert!(held.init_mode());
        held.advance_to(20);
        // The forced zero drains the top bit, which a free-running poly does
        // not do — so the two configurations must diverge.
        let mut free = running();
        free.advance_to(20);
        assert_ne!(
            held.poly.shift, free.poly.shift,
            "init mode must not behave like the running poly"
        );

        // And CST.MAC:18 releases it.
        held.write(reg::SKCTL, 0x07, 20);
        assert!(!held.init_mode());
        let before = held.read(reg::RANDOM, 20);
        assert_ne!(held.read(reg::RANDOM, 120), before, "now free-running");
    }

    #[test]
    fn the_sequence_is_deterministic_from_reset() {
        // Two identically driven chips must agree exactly — the whole point of
        // a reproducible headless run.
        let mut a = running();
        let mut b = running();
        for c in (0..10_000).step_by(37) {
            assert_eq!(a.read(reg::RANDOM, c), b.read(reg::RANDOM, c));
        }
    }

    #[test]
    fn random_is_well_spread() {
        // Not a statistical test — just a guard that it is not stuck or
        // near-constant, which is exactly how a stubbed RANDOM would look.
        let mut p = running();
        let mut seen = [false; 256];
        for c in 1..=20_000u64 {
            seen[p.read(reg::RANDOM, c) as usize] = true;
        }
        let distinct = seen.iter().filter(|&&s| s).count();
        assert_eq!(distinct, 256, "every byte value should occur");
    }

    #[test]
    fn writable_registers_are_stored() {
        let mut p = Pokey::new();
        for i in 0..16 {
            p.write(i, 0x10 + i as u8, 0);
        }
        for i in 0..16 {
            assert_eq!(p.regs[i], 0x10 + i as u8);
        }
    }

    #[test]
    fn pots_read_idle() {
        let mut p = running();
        for i in 0..8 {
            assert_eq!(p.read(i, 10), 0, "POT{i}");
        }
        assert_eq!(p.read(reg::ALLPOT, 10), 0, "no pot still counting");
        assert_eq!(p.read(reg::IRQST, 10), 0xFF, "active low: nothing pending");
    }
}

#[cfg(test)]
mod poly_tests {
    use super::*;

    /// The poly4 register, recovered by un-decimating what the core emitted at
    /// `AUDC = 0xC`. Two sampling rates, 28 cycles and 4, produce this same
    /// sequence up to rotation.
    const POLY4_MEASURED: [u8; 15] = [1, 1, 0, 1, 0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0];

    /// The poly5 register, from the *transitions* of the core's output at
    /// `AUDC = 0x2` — poly5 only gates, so the edges carry it — un-decimated the
    /// same way and cross-checked against the same two sampling rates.
    const POLY5_MEASURED: [u8; 31] = [
        1, 1, 1, 1, 0, 1, 1, 0, 1, 0, 0, 1, 1, 0, 0, 0, 0, 0, 1, 1, 1, 0, 0, 1, 0, 0, 0, 1, 0, 1,
        0,
    ];

    fn run<F: FnMut() -> u8>(mut next: F, n: usize) -> Vec<u8> {
        (0..n).map(|_| next()).collect()
    }

    #[test]
    fn poly4_reproduces_the_measured_sequence() {
        let mut p = Poly4::new();
        let got = run(
            || {
                let b = p.bit() as u8;
                p.step();
                b
            },
            45,
        );
        for (i, chunk) in got.chunks(15).enumerate() {
            assert_eq!(chunk, POLY4_MEASURED, "poly4 period {i}");
        }
    }

    #[test]
    fn poly5_reproduces_the_measured_sequence() {
        let mut p = Poly5::new();
        let got = run(
            || {
                let b = p.bit() as u8;
                p.step();
                b
            },
            93,
        );
        for (i, chunk) in got.chunks(31).enumerate() {
            assert_eq!(chunk, POLY5_MEASURED, "poly5 period {i}");
        }
    }

    /// The ones-counts are what pinned each polynomial's feedback polarity, and
    /// poly5's odd count is what explains its output period being 62 ticks
    /// rather than 31. If either changed, the identification would be wrong.
    #[test]
    fn the_ones_counts_are_the_ones_that_identified_the_polarity() {
        assert_eq!(POLY4_MEASURED.iter().filter(|&&b| b == 1).count(), 8, "XOR");
        assert_eq!(POLY5_MEASURED.iter().filter(|&&b| b == 1).count(), 15, "XNOR");
    }

    /// Both are maximal: every non-excluded state appears exactly once.
    #[test]
    fn both_polynomials_are_maximal_length() {
        let mut p4 = Poly4::new();
        let mut seen4 = std::collections::BTreeSet::new();
        for _ in 0..15 {
            assert!(seen4.insert(p4.shift), "poly4 repeated a state early");
            p4.step();
        }
        assert_eq!(p4.shift, Poly4::RESET, "poly4 closed the cycle at 15");
        assert!(!seen4.contains(&0), "XOR cannot reach the all-zero state");

        let mut p5 = Poly5::new();
        let mut seen5 = std::collections::BTreeSet::new();
        for _ in 0..31 {
            assert!(seen5.insert(p5.shift), "poly5 repeated a state early");
            p5.step();
        }
        assert_eq!(p5.shift, Poly5::RESET, "poly5 closed the cycle at 31");
        assert!(!seen5.contains(&0x1F), "XNOR cannot reach the all-ones state");
    }

    /// The divisors are measured; this records the relationship that confirmed
    /// they are POKEY's two audio clocks and not some other pair of numbers.
    #[test]
    fn the_two_divisors_are_the_documented_clock_pair() {
        assert_eq!(DIV_FAST, 28);
        assert_eq!(DIV_SLOW, 114);
        let ratio = f64::from(DIV_SLOW) / f64::from(DIV_FAST);
        assert!((ratio - 63.9 / 15.7).abs() < 0.02, "ratio {ratio} is 64k:15k");
    }
}

#[cfg(test)]
mod audio_tests {
    use super::*;

    /// Set up one chip, strobe STIMER, and take `snd` on every cycle after.
    fn stream(setup: &[(usize, u8)], cycles: u64) -> Vec<u8> {
        let mut p = Pokey::new();
        p.write(reg::SKCTL, 0x07, 0); // what CST.MAC:18 writes
        for &(i, v) in setup {
            p.write(i, v, 0);
        }
        p.write(reg::STIMER, 0, 0);
        (1..=cycles)
            .map(|c| {
                p.advance_to(c);
                p.snd()
            })
            .collect()
    }

    /// The period of the settled stream. The first divider window is a
    /// transient — every output flip-flop starts cleared, where in steady state
    /// it would hold whatever the previous sample left — so the leading third is
    /// discarded before measuring.
    fn period(v: &[u8]) -> Option<usize> {
        let t = &v[v.len() / 3..];
        (1..=(t.len() / 2)).find(|&p| t[..t.len() - p].iter().zip(&t[p..]).all(|(a, b)| a == b))
    }

    /// The periods the core produced for each distortion, reproduced by our own
    /// pipeline with no oracle present.
    ///
    /// These are the figures from the sweep in [`Poly5`]'s docs, in CPU cycles:
    /// one channel, `AUDF = 0`, full volume. Getting them out of an
    /// independently written divider, gate and flip-flop is the strongest check
    /// available on this side of the seam — task #62 is what compares the actual
    /// samples rather than their periods.
    #[test]
    fn each_distortion_has_the_period_measured_off_the_core() {
        for (audc, want) in [(0xAFu8, 56usize), (0xEF, 56), (0xCF, 420), (0x2F, 1736), (0x6F, 1736)]
        {
            let s = stream(&[(0, 0), (1, audc)], 12_000);
            assert_eq!(period(&s), Some(want), "AUDC {audc:02X}");
        }
    }

    /// poly4 against poly5 beats at the least common multiple of 15 and 31.
    #[test]
    fn poly4_and_poly5_together_beat_at_their_lcm() {
        let s = stream(&[(0, 0), (1, 0x4F)], 60_000);
        assert_eq!(period(&s), Some(465 * 28));
    }

    /// The two settings that measured identical on the core must be identical
    /// here too — they are what showed AUDC bit 7 is the poly5 selector.
    #[test]
    fn the_duplicate_distortions_are_actually_duplicates() {
        for (a, b) in [(0x2Fu8, 0x6Fu8), (0xAF, 0xEF)] {
            let sa = stream(&[(0, 0), (1, a)], 4_000);
            let sb = stream(&[(0, 0), (1, b)], 4_000);
            assert_eq!(sa, sb, "AUDC {a:02X} vs {b:02X}");
        }
    }

    #[test]
    fn all_zero_registers_are_silent() {
        let s = stream(&[], 4_000);
        assert!(s.iter().all(|&v| v == 0), "a chip nobody programmed makes no sound");
    }

    /// AUDC bit 4 emits the level continuously, ignoring the flip-flop.
    #[test]
    fn volume_only_is_a_constant_level() {
        let s = stream(&[(0, 0), (1, 0x1F)], 4_000);
        assert!(s.iter().all(|&v| v == 15), "volume-only holds its level");

        // And it is the level, not just "nonzero".
        let s = stream(&[(0, 0), (1, 0x15)], 100);
        assert!(s.iter().all(|&v| v == 5));
    }

    /// A pure tone spends equal time high and low, at the volume asked for.
    #[test]
    fn a_pure_tone_is_a_square_wave_at_the_programmed_volume() {
        let s = stream(&[(0, 0), (1, 0xA9)], 5_600);
        let high = s.iter().filter(|&&v| v == 9).count();
        let low = s.iter().filter(|&&v| v == 0).count();
        assert_eq!(high + low, s.len(), "only two levels");
        assert_eq!(high, low, "equal mark and space");
    }

    /// The divider is AUDF + 1 base ticks, so the period grows linearly.
    #[test]
    fn the_period_follows_audf_plus_one() {
        for audf in [0u8, 1, 3, 7] {
            let want = 2 * (usize::from(audf) + 1) * DIV_FAST as usize;
            let s = stream(&[(0, audf), (1, 0xAF)], (want * 4) as u64);
            assert_eq!(period(&s), Some(want), "AUDF {audf}");
        }
    }

    /// AUDCTL bit 0 switches every channel to the slow base clock.
    #[test]
    fn audctl_bit_0_selects_the_slow_clock() {
        let s = stream(&[(reg::AUDCTL, 0x01), (0, 0), (1, 0xAF)], 4_000);
        assert_eq!(period(&s), Some(2 * DIV_SLOW as usize));
    }

    /// AUDCTL bit 6 puts channel 1 on the main clock, where the divider is
    /// AUDF + 4 *cycles* — both figures measured off the core.
    #[test]
    fn audctl_bit_6_puts_channel_1_on_the_main_clock() {
        for audf in [0u8, 4] {
            let want = 2 * (usize::from(audf) + 4);
            let s = stream(&[(reg::AUDCTL, 0x40), (0, audf), (1, 0xAF)], 400);
            assert_eq!(period(&s), Some(want), "AUDF {audf}");
        }
    }

    /// STIMER restarts the dividers together, so two channels programmed alike
    /// stay in step and their levels add rather than beating.
    #[test]
    fn stimer_synchronises_the_channels() {
        let mut p = Pokey::new();
        p.write(reg::SKCTL, 0x07, 0);
        p.write(0, 0, 0);
        p.write(1, 0xA7, 0); // channel 0, pure tone, volume 7
        // Channel 1 set up later, so its divider is out of phase to begin with.
        p.advance_to(37);
        p.write(2, 0, 37);
        p.write(3, 0xA7, 37);

        let mut before = false;
        for c in 38..500u64 {
            p.advance_to(c);
            if p.snd() == 7 {
                before = true; // exactly one channel high: they disagree
            }
        }
        assert!(before, "without a strobe the two channels are out of phase");

        p.write(reg::STIMER, 0, 500);
        for c in 501..2_000u64 {
            p.advance_to(c);
            assert!(p.snd() == 0 || p.snd() == 14, "after STIMER they move together");
        }
    }

    /// Two chips sum on this board: `SOUT = {2'b00,snd1} + {2'b00,snd2}`
    /// (`AudioOutput.v:51`). That is a wiring fact about how the pair is
    /// combined, not chip internals.
    #[test]
    fn the_board_sums_both_chips() {
        let mut a = Pokey::new();
        let mut b = Pokey::new();
        for p in [&mut a, &mut b] {
            p.write(reg::SKCTL, 0x07, 0);
            p.write(1, 0x1F, 0); // volume-only, full
        }
        a.advance_to(100);
        b.advance_to(100);
        assert_eq!(u16::from(a.snd()) + u16::from(b.snd()), 30);
    }

    /// All four channels at full volume-only is the loudest one chip goes, and
    /// it must fit the six bits the chip presents.
    #[test]
    fn snd_is_six_bits_wide() {
        let s = stream(&[(1, 0x1F), (3, 0x1F), (5, 0x1F), (7, 0x1F)], 100);
        assert!(s.iter().all(|&v| v == 60), "4 x 15");
        assert!(60 < 64, "fits in six bits");
    }
}
