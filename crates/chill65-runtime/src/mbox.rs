//! Tempest's math box, modelled functionally.
//!
//! A second processor: four cascaded AMD 2901 bit-slices making a 16-bit ALU,
//! sequenced by its own microcode ROM, sitting on the aux board beside the
//! 6502. `atari-recompiler-plan.md` §6 settled how to approach it — implement
//! the operations, do not execute `MBUCOD.V05`'s microcode, which is what MAME
//! does and is almost certainly right.
//!
//! The interface is 32 addresses, `ALCOMN.MAC:287-311` placing them at
//! `MBSTAR=6080` for writes with status and output reads at `6040`, `6060` and
//! `6070`. Every function below is `MBUDOC.DOC:23-73`, Atari's own overview,
//! which is unusually generous: it names each offset, says what each starts,
//! and gives per-operation timings in cycles.
//!
//! # Why nothing is ever busy
//!
//! `MBUDOC.DOC:27` has `STAT` reading busy in D7, and the operations are
//! quoted at 10 to 131+5N cycles. Nothing in the corpus ever *requires*
//! observing busy=1: every consumer either polls with a `BMI`-shaped loop,
//! which falls straight through when the flag never sets, or ignores `STAT`
//! altogether.
//!
//! The binding constraint runs the other way. `ALTEST.MAC:784-799`'s `READMB`
//! allows 100 decimal poll iterations before declaring a timeout, and
//! `ALTEST.MAC:767-768` treats an exhausted counter as a failed math box. So
//! an operation that took too long would fail the game's own diagnostic, while
//! one that completes instantly cannot. Operations therefore complete inside
//! the write that starts them and [`MathBox::stat`] always reads done.
//!
//! **UNVERIFIED:** that no timing-sensitive path exists that this flattens.
//! The plan (§6) records the general risk for this class of coprocessor — the
//! 6502 sometimes delaying a fixed number of cycles rather than polling — and
//! `MBUDOC.DOC`'s cycle counts are kept in the comments below so the question
//! can be reopened against real numbers rather than from memory.

/// The 32-address window, as offsets from `MBSTAR`. `MBUDOC.DOC:25-67`.
mod off {
    pub const AL: u8 = 0x00;
    pub const AH: u8 = 0x01;
    pub const BL: u8 = 0x02;
    pub const BH: u8 = 0x03;
    pub const EL: u8 = 0x04;
    pub const EH: u8 = 0x05;
    pub const FL: u8 = 0x06;
    pub const FH: u8 = 0x07;
    pub const XL: u8 = 0x08;
    pub const XH: u8 = 0x09;
    pub const YL: u8 = 0x0A;
    /// Write high byte of Y, start multiply. [54-59]
    pub const YHSM: u8 = 0x0B;
    /// Number of bits in the quotient.
    pub const NL: u8 = 0x0C;
    pub const ZLL: u8 = 0x0D;
    pub const ZLH: u8 = 0x0E;
    pub const ZHL: u8 = 0x0F;
    pub const ZHH: u8 = 0x10;
    /// Write high byte of Y, start the whole mess. [119-131+5N]
    pub const YHSP: u8 = 0x11;
    /// Any write starts the Y' multiply. [50-55]
    pub const SYM: u8 = 0x12;
    /// Any write starts the Y'/X' divide. [11-13+5N]
    pub const SYXD: u8 = 0x13;
    /// Any write starts the Z/X' divide. [10-12+5N]
    pub const SZXD: u8 = 0x14;
    pub const XPL: u8 = 0x15;
    pub const XPH: u8 = 0x16;
    /// Any write puts X' on the output.
    pub const DXP: u8 = 0x17;
    /// Any write puts Y'(low 16) on the output.
    pub const DYPL: u8 = 0x18;
    /// Any write puts Y'(high 16) on the output.
    pub const DYPH: u8 = 0x19;
    pub const YPL: u8 = 0x1A;
    pub const YPH: u8 = 0x1B;
    /// Write high byte of Y, start clipping.
    pub const YHSC: u8 = 0x1C;
}

/// The 2901 register file and output bus.
///
/// There is no result register: `MBUDOC.DOC:30-33` reads `YLOW`/`YHIGH` off
/// the 2901 output bus, so what a read returns is whatever the last operation
/// left driving it. That includes plain register writes — `MBUDOC.DOC:70-73`
/// notes every `*` address presents the just-modified register at the output
/// "to verify correct operation… probably useful only in self-test", which is
/// exactly where the corpus uses it.
#[derive(Clone, Debug, Default)]
pub struct MathBox {
    a: u16,
    b: u16,
    e: u16,
    f: u16,
    x: u16,
    y: u16,
    /// Bits of quotient the divider produces. `MBUDOC.DOC:44`.
    n: u8,
    /// The 32-bit dividend, written a byte at a time, LSB first.
    z: u32,
    xp: u16,
    /// Y' is 32 bits: `DYPL` and `DYPH` read its halves separately.
    yp: u32,
    /// The 2901 output bus.
    out: u16,
}

impl MathBox {
    pub fn new() -> Self {
        Self::default()
    }

    /// Busy/done. D7=1 would be busy; see this module's header for why it
    /// never is. `MBUDOC.DOC:27`.
    pub fn stat(&self) -> u8 {
        0x00
    }

    /// Low byte of the 2901 output. `MBUDOC.DOC:30`.
    pub fn ylow(&self) -> u8 {
        self.out as u8
    }

    /// High byte of the 2901 output. `MBUDOC.DOC:32`.
    pub fn yhigh(&self) -> u8 {
        (self.out >> 8) as u8
    }

    /// Write one of the 32 addresses. `offset` is masked to the window.
    pub fn write(&mut self, offset: u8, value: u8) {
        let v = value as u16;
        let lo = |r: u16, v: u16| (r & 0xFF00) | v;
        let hi = |r: u16, v: u16| (r & 0x00FF) | (v << 8);

        // Every `*` address leaves the register it just changed on the output.
        macro_rules! load {
            ($reg:ident, $half:expr) => {{
                self.$reg = $half;
                self.out = self.$reg;
            }};
        }

        match offset & 0x1F {
            off::AL => load!(a, lo(self.a, v)),
            off::AH => load!(a, hi(self.a, v)),
            off::BL => load!(b, lo(self.b, v)),
            off::BH => load!(b, hi(self.b, v)),
            off::EL => load!(e, lo(self.e, v)),
            off::EH => load!(e, hi(self.e, v)),
            off::FL => load!(f, lo(self.f, v)),
            off::FH => load!(f, hi(self.f, v)),
            off::XL => load!(x, lo(self.x, v)),
            off::XH => load!(x, hi(self.x, v)),
            off::YL => load!(y, lo(self.y, v)),

            // Write high Y *and* start: the high byte lands first, so the
            // multiply sees the value this very write completes.
            // `MBUDOC.DOC:41-43`: output = (x-e)*a - (y-f)*b.
            off::YHSM => {
                self.y = hi(self.y, v);
                let r = mul(self.x, self.e, self.a) - mul(self.y, self.f, self.b);
                self.out = r as u16;
            }

            off::NL => self.n = value,

            off::ZLL => self.z = byte_of(self.z, 0, value),
            off::ZLH => self.z = byte_of(self.z, 1, value),
            off::ZHL => self.z = byte_of(self.z, 2, value),
            off::ZHH => self.z = byte_of(self.z, 3, value),

            // `MBUDOC.DOC:49-51`: output = (xb + ya + f) / (xa - yb + e).
            //
            // **UNVERIFIED.** The corpus never writes this offset, so the
            // fixed-point alignment of the divide is untested. Modelled as
            // written, with the same divider the other two use.
            off::YHSP => {
                self.y = hi(self.y, v);
                let num = smul(self.x, self.b) + smul(self.y, self.a) + self.f as i32 as i64;
                let den = smul(self.x, self.a) - smul(self.y, self.b) + self.e as i32 as i64;
                self.out = divide(num as u64, den as u16, self.n);
            }

            // `MBUDOC.DOC:52-54`: output = (x-e)b + (y-f)a. This is the one
            // the display code actually starts — `ALDISP.MAC:2081` writes SYM
            // — and the result is Y', which `SYXD` then divides.
            off::SYM => {
                let r = mul(self.x, self.e, self.b) + mul(self.y, self.f, self.a);
                self.yp = r as u32;
                self.out = r as u16;
            }

            // `MBUDOC.DOC:55`: Y'/X'.
            off::SYXD => self.out = divide(self.yp as u64, self.xp, self.n),

            // `MBUDOC.DOC:56`: Z/X'. The one the self-test pins, and the one
            // `WORSCR` and `CASCAL` use.
            off::SZXD => self.out = divide(self.z as u64, self.xp, self.n),

            off::XPL => load!(xp, lo(self.xp, v)),
            off::XPH => load!(xp, hi(self.xp, v)),

            off::DXP => self.out = self.xp,
            off::DYPL => self.out = self.yp as u16,
            off::DYPH => self.out = (self.yp >> 16) as u16,

            // **UNVERIFIED:** `MBUDOC.DOC:64-65` calls these the low and high
            // *bytes* of Y' while `DYPL`/`DYPH` read 16-bit halves, so they are
            // taken as the low and high bytes of Y''s low half. The corpus
            // never writes them.
            off::YPL => self.yp = (self.yp & 0xFFFF_FF00) | value as u32,
            off::YPH => self.yp = (self.yp & 0xFFFF_00FF) | ((value as u32) << 8),

            // **UNVERIFIED:** `MBUDOC.DOC:66` names the clipping start and
            // says nothing about what it computes, and no `MBUCOD` listing of
            // it survives in a form this can be written from. The high Y byte
            // lands, which is documented; the output is left alone rather than
            // invented. The corpus never writes this offset.
            off::YHSC => self.y = hi(self.y, v),

            // `MBUDOC.DOC:67` — "currenty unused". They must still be
            // *accepted*: `ALTEST.MAC:524-548`'s signature scan writes every
            // one of the 32 addresses with junk. `MBUCOD.V05` maps two of them
            // to distance routines the documentation predates, which is worth
            // knowing and not worth guessing at.
            _ => {}
        }
    }
}

fn byte_of(word: u32, index: u32, value: u8) -> u32 {
    let shift = index * 8;
    (word & !(0xFFu32 << shift)) | ((value as u32) << shift)
}

/// `(p - q) * r`, all signed 16-bit, to a signed 32-bit product.
fn mul(p: u16, q: u16, r: u16) -> i32 {
    ((p as i16 as i32) - (q as i16 as i32)) * (r as i16 as i32)
}

/// A plain signed 16x16 product, widened.
fn smul(p: u16, q: u16) -> i64 {
    (p as i16 as i64) * (q as i16 as i64)
}

/// The divider: `n` iterations of shift-subtract, as the hardware runs it.
///
/// Modelled as a loop rather than as a closed form because that is what the
/// 2901 does — `MBUDOC.DOC:55-56` prices both divides at `5N` cycles plus a
/// fixed setup, which is a bit per iteration and nothing else. Writing it this
/// way means the `N` the caller chooses decides the alignment, exactly as it
/// does in hardware, instead of the model having to know which of the game's
/// three call sites it is serving.
///
/// The self-test pins the `N=16` case: `ALTEST.MAC:759-799` loads X' and Z
/// with the same value and requires the quotient to be exactly 1.
///
/// **It also pins the divide as unsigned.** `MBTEST` walks its operand upward
/// through every 16-bit value — `ALTEST.MAC:777-780` increments `TEMPX` and
/// carries into `TEMPY` — so it eventually divides `8000` by `8000` and still
/// demands 1. Read as two's complement that is `+32768 / -32768`, which is
/// -1, and `ALTEST.MAC:770` would fail the box. The 2901 has sign control and
/// the microcode could use it, but for this operation the game's own
/// diagnostic settles the question.
///
/// **UNVERIFIED:** whether the other two divides are unsigned as well. They
/// have no oracle short of running the game.
///
/// **UNVERIFIED:** the `N=0x0F` alignment `WORSCR` uses
/// (`ALDISP.MAC:2208-2310`) and the `N=0x18` fractional one from `CASCAL`
/// (`ALDISP.MAC:1444-1477`). Neither has an oracle short of running the game,
/// which is what `boot_te` and `attract_te` are for.
fn divide(dividend: u64, divisor: u16, n: u8) -> u16 {
    if divisor == 0 {
        // No documented behaviour, and the game guards its divisors. Zero is
        // returned rather than trapping, because a panic here would turn a
        // game bug into a crashed emulator.
        return 0;
    }
    // N is a bit count held in a byte, and `ALTEST.MAC:524-548`'s signature
    // scan writes junk to every address including this one — 188 bits of
    // quotient is not a request, it is noise, and it must not fault.
    let n = n.min(32);
    let mut rem: u64 = 0;
    let d = divisor as u64;
    let mag = dividend;
    let mut quotient: u32 = 0;

    // `N` iterations consume the dividend's low `N` bits, most significant of
    // those first. The self-test fixes this alignment: it puts a 16-bit value
    // in Z with the high half zeroed, the same value in X', asks for N=16 and
    // requires the answer 1 — which only comes out if those sixteen bits are
    // the ones the divider sees. Scaling above that is the caller's job, and
    // both other call sites do it: `WORSCR` pre-shifts its dividend left by 8
    // and `CASCAL` asks for N=0x18, each buying fractional bits by choosing
    // what to feed in rather than by changing what the divider does.
    for i in 0..n as u32 {
        let bit = (mag >> (n as u32 - 1 - i)) & 1;
        rem = (rem << 1) | bit;
        quotient <<= 1;
        if rem >= d {
            rem -= d;
            quotient |= 1;
        }
    }

    quotient as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the self-test's own sequence: `ALTEST.MAC:759-799`.
    ///
    /// `MBTEST` puts the same 16-bit value in X' and in the low half of Z,
    /// zeroes the high half, sets N to 0x10 and starts the Z/X' divide. It
    /// then demands `MYLOW == 1` and `MYHIGH == 0` — anything else increments
    /// `MBCOND` and fails the diagnostic.
    fn self_test_divide(value: u16) -> (u8, u8) {
        let mut mb = MathBox::new();
        let (lo, hi) = (value as u8, (value >> 8) as u8);
        mb.write(off::XPL, lo);
        mb.write(off::ZLL, lo);
        mb.write(off::XPH, hi);
        mb.write(off::ZLH, hi);
        mb.write(off::ZHL, 0);
        mb.write(off::ZHH, 0);
        mb.write(off::NL, 0x10);
        mb.write(off::SZXD, 0);
        (mb.ylow(), mb.yhigh())
    }

    #[test]
    fn the_games_own_divide_vector_answers_one() {
        // MBTEST walks TEMPX/TEMPY upwards, so it is not one value but every
        // value the game reaches; a handful of them, including the edges it
        // would hit first and last.
        for v in [1u16, 2, 3, 0x00FF, 0x0100, 0x1234, 0x7FFF, 0x8000, 0xFFFF] {
            assert_eq!(
                self_test_divide(v),
                (1, 0),
                "dividing {v:#06x} by itself must give 1, or MBCOND increments"
            );
        }
    }

    #[test]
    fn never_busy_so_the_poll_loop_falls_through() {
        // READMB allows 100 decimal iterations; reporting done immediately
        // means the loop exits on its first read with the counter untouched.
        let mut mb = MathBox::new();
        mb.write(off::NL, 0x10);
        mb.write(off::SZXD, 0);
        assert_eq!(mb.stat() & 0x80, 0, "D7 clear is done");
    }

    #[test]
    fn a_register_write_puts_that_register_on_the_output() {
        // MBUDOC.DOC:70-73 — the readback the self-test relies on.
        let mut mb = MathBox::new();
        mb.write(off::AL, 0x34);
        mb.write(off::AH, 0x12);
        assert_eq!((mb.ylow(), mb.yhigh()), (0x34, 0x12), "A reads back");

        mb.write(off::XPL, 0xCD);
        mb.write(off::XPH, 0xAB);
        assert_eq!((mb.ylow(), mb.yhigh()), (0xCD, 0xAB), "X' reads back");

        // And DXP puts it back after something else has driven the bus.
        mb.write(off::BL, 0x99);
        mb.write(off::DXP, 0);
        assert_eq!((mb.ylow(), mb.yhigh()), (0xCD, 0xAB));
    }

    #[test]
    fn the_y_prime_multiply_is_the_documented_expression() {
        // MBUDOC.DOC:52-54 — output = (x-e)b + (y-f)a, and the result is Y',
        // which DYPL/DYPH read back.
        let mut mb = MathBox::new();
        let load = |mb: &mut MathBox, lo_off, hi_off, v: u16| {
            mb.write(lo_off, v as u8);
            mb.write(hi_off, (v >> 8) as u8);
        };
        load(&mut mb, off::XL, off::XH, 10);
        load(&mut mb, off::EL, off::EH, 4);
        load(&mut mb, off::BL, off::BH, 3);
        load(&mut mb, off::YL, off::YL, 0);
        load(&mut mb, off::FL, off::FH, 2);
        load(&mut mb, off::AL, off::AH, 5);
        mb.write(off::SYM, 0);
        // (10-4)*3 + (0-2)*5 = 18 - 10 = 8
        assert_eq!(mb.ylow(), 8);
        assert_eq!(mb.yhigh(), 0);
        mb.write(off::DYPL, 0);
        assert_eq!(mb.ylow(), 8, "Y' kept the product");
    }

    #[test]
    fn the_signature_scan_writes_every_offset_without_upsetting_it() {
        // ALTEST.MAC:524-548 writes junk to all 32 addresses, including the
        // three MBUDOC calls unused. Nothing may panic, and the box must still
        // answer afterwards.
        let mut mb = MathBox::new();
        for offset in 0..32u8 {
            mb.write(offset, offset.wrapping_mul(37));
        }
        assert_eq!(mb.stat() & 0x80, 0);
        assert_eq!(self_test_divide(0x1234), (1, 0), "still divides correctly");
    }

    #[test]
    fn a_zero_divisor_returns_rather_than_trapping() {
        let mut mb = MathBox::new();
        mb.write(off::NL, 0x10);
        mb.write(off::SZXD, 0);
        assert_eq!((mb.ylow(), mb.yhigh()), (0, 0));
    }
}
