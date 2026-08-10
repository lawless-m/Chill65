//! POKEY ×2 — registers, the RANDOM generator, and the audio polynomials.
//!
//! # Scope
//!
//! Audio output was out of scope for Phase 2 (plan §5 calls POKEY "not the
//! interesting part", and the gate is headless). It is being built now, from the
//! bottom up: this module holds the free-running parts — [`Poly4`], [`Poly5`]
//! and the two clock divisors — and the per-channel pipeline that consumes them
//! is not written yet.
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
//! One thing this does *not* yet cover: poly17's audio tap. [`Poly17`] below
//! predates this work and drives `RANDOM`, where its sourcing is noted on the
//! type itself.
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

/// The 4-bit polynomial counter.
///
/// **Measured, not transcribed.** `rtl/Pokey/` is © 2013 Mark Watson and
/// licensed non-commercially, so nothing here comes from it; see the README.
/// Instead a fixture selected distortion `AUDC = 0xC` — poly4 with poly5 out of
/// circuit — with `AUDF = 0` and full volume, and the core's `SOUT` was sampled
/// once per divider tick. That setting puts the polynomial's bit straight on the
/// output, so the sequence is read directly:
///
/// ```text
/// 1,0,0,0,1,1,1,1,0,1,0,1,1,0,0     period 15, 8 ones
/// ```
///
/// A brute-force search over every tap subset, both feedback polarities, both
/// shift directions and every start state found **exactly one** machine
/// reproducing it, up to the mirror convention: taps 0 and 3, XOR, shifting
/// right. Eight ones in fifteen is the XOR signature (XOR excludes the all-zero
/// state), which agrees independently.
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
    /// The state at the start of the measured window.
    ///
    /// This is **not** established to be the state at reset — the fixture idles
    /// for tens of thousands of cycles before the window, so only the phase
    /// within the sequence was observed, not its origin. Aligning the stream to
    /// the core's absolute phase is what task #62's comparison does.
    pub const RESET: u8 = 0b0001;

    pub fn new() -> Self {
        Poly4 { shift: Self::RESET }
    }

    pub fn step(&mut self) {
        let fb = ((self.shift) ^ (self.shift >> 3)) & 1;
        self.shift = ((self.shift >> 1) | (fb << 3)) & 0x0F;
    }

    pub fn bit(&self) -> bool {
        self.shift & 1 != 0
    }
}

/// The 5-bit polynomial counter.
///
/// **Measured, not transcribed**, by the same method as [`Poly4`] — but poly5
/// cannot be read directly, and establishing that was most of the work.
///
/// Sweeping all eight distortion settings showed `0x2` and `0x6` producing
/// identical output, and `0xA` and `0xE` likewise. That is the documented shape
/// of `AUDC` bits 7:5 confirmed by measurement: **bit 7 selects whether poly5 is
/// in the chain**, bits 6:5 pick poly17, poly4 or a pure tone after it. So no
/// setting puts poly5's bit on the output; it only *gates* the stage behind it:
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
/// 62 rather than 31 because the gated output *toggles*: an odd number of gate
/// events per polynomial period doubles it. So poly5's sequence is in the
/// output's **transitions**, and those recover it:
///
/// ```text
/// 1,0,0,0,1,0,0,0,0,0,1,1,0,1,1,0,0,1,1,1,1,0,1,0,0,1,0,1,0,1,1
/// period 31, 15 ones
/// ```
///
/// Fifteen ones is odd, which is what predicted the 62-tick period, and it is
/// also the XNOR signature (XNOR excludes the all-ones state; XOR would give
/// 16). The search returned one machine in two mirror forms — taps 0,1,2,3 XNOR
/// shifting right, equivalently taps 1,2,3,4 shifting left.
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
    /// The state at the start of the measured window; see [`Poly4::RESET`] for
    /// why this is a phase and not an origin.
    pub const RESET: u8 = 0b10001;

    pub fn new() -> Self {
        Poly5 { shift: Self::RESET }
    }

    pub fn step(&mut self) {
        let fb = !(self.shift & 0b0_1111).count_ones() as u8 & 1;
        self.shift = ((self.shift >> 1) | (fb << 4)) & 0x1F;
    }

    pub fn bit(&self) -> bool {
        self.shift & 1 != 0
    }
}

/// Register indices within a chip.
pub mod reg {
    pub const AUDCTL: usize = 0x08;
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

    /// Bring the poly up to `cycle`, one step per CPU cycle.
    pub fn advance_to(&mut self, cycle: u64) {
        let select9 = self.select9();
        let init = self.init_mode();
        for _ in self.advanced_to..cycle {
            self.poly.step(select9, init);
        }
        self.advanced_to = self.advanced_to.max(cycle);
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
        self.regs[index & 0x0F] = value;
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

    /// The sequence read straight off the core's `SOUT` at `AUDC = 0xC`.
    const POLY4_MEASURED: [u8; 15] = [1, 0, 0, 0, 1, 1, 1, 1, 0, 1, 0, 1, 1, 0, 0];

    /// The transitions of the core's `SOUT` at `AUDC = 0x2`, one per divider
    /// tick. Not the output level — poly5 only gates, so the sequence is in the
    /// edges.
    const POLY5_MEASURED: [u8; 31] = [
        1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 1, 0, 1, 1, 0, 0, 1, 1, 1, 1, 0, 1, 0, 0, 1, 0, 1, 0, 1,
        1,
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
