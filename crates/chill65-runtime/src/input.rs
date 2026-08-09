//! Switches and trackball, as the CPU sees them.
//!
//! Host-facing API only — there is no event loop, no window and no third-party
//! crate. A front end calls [`Input::set_switch`] and
//! [`Input::add_trackball_delta`]; the machine does the rest.
//!
//! # Address map
//!
//! `IN0n` covers `9400-97FF` and `CCastles.v:114-117` splits it on `BA[9]`:
//!
//! | Window | Contents |
//! |---|---|
//! | `9400`–`95FF` | LETA trackball interface, `addr = BA[1:0]` (`CCastles.v:294`) |
//! | `9600`–`97FF` | player switches |
//!
//! Only two of the LETA's four axes are wired (`CCastles.v:292`):
//! `.X1(tb1VD), .Y1(tb1VC)` is **vertical** and `.X2(tb1HD), .Y2(tb1HC)` is
//! **horizontal**, so register 0 is vertical and register 1 horizontal. The
//! game agrees: `HW.TBV = 9400`, `HW.TBH = 9401` (`CG.MAC:120-121`).
//!
//! Watch the axis order. `CG.MAC` contains *two* equate blocks under
//! conditional assembly and they **swap the axes**. `CG.MAC:5` sets
//! `CG.PC = 1` ("PC = 0 wirewrap, 1 PC board"), so `.IF EQ,CG.PC` is false and
//! the production block at `:120` is the live one. The wirewrap block at `:62`
//! has them the other way round and would put vertical motion on the horizontal
//! axis.
//!
//! # The switch byte
//!
//! `CCastles.v:102`:
//!
//! ```verilog
//! wire [7:0] playerSwitches =
//!     { ~STARTJMP2, ~STARTJMP1, VBLANK, ~SELFTEST, ~SLAM, ~COINA, ~COINL, ~COINR};
//! ```
//!
//! Every bit is active-low **except bit 5, which is VBLANK undirected** — a
//! live video signal, not a switch. The game's equates corroborate all of it
//! independently: `MA.VBL = MD5`, `MA.STS = MD4` (`CG.MAC:123,134`), and
//! `COIN01 = 1 ; COINS ARE IN D0-D2` with `COIN = 0 ; COIN SWITCHES ARE ACTIVE
//! LOW` (`CG.MAC:334-336`).
//!
//! Note that start and jump are the *same physical input* — the RTL calls it
//! `STARTJMP`, and the game maps `MA.ST1` and `MA.BUT` both to `MD6`
//! (`CG.MAC:125,127`). One button, two meanings depending on game state.
//!
//! # Trackball: the counter wraps, the *game* saturates
//!
//! `SupportChips.v:297-307` — the quadrature decoder is a **free-running 9-bit
//! counter with no saturation and no reset**, exposed as `count = counter[8:1]`:
//!
//! ```verilog
//! reg [8:0] counter;
//! if(ce) begin if(dir) counter <= counter + 1; else counter <= counter - 1; end
//! assign count = counter[8:1];
//! ```
//!
//! It is an **absolute position**, not a delta, and the game differences it
//! itself (`CEN.MAC:317-330`):
//!
//! ```text
//! TR.DEL:  LDA HW.TBH
//!          SUB TR.I     ; delta = current - previous
//!          STA TR.XD
//!          ...
//!          LDA HW.TBH
//!          STA TR.I     ; remember for next time
//! ```
//!
//! The 8-bit subtraction wraps, which is precisely why a wrapping counter is
//! correct: the wrap is invisible to the game so long as movement between
//! samples stays under 128 counts. Clamping is then done **in software** by
//! `GP.TRC` ("truncates to -TEMP1:TEMP1", `CEN.MAC:338-350`).
//!
//! So: **do not saturate here.** A hardware clamp would corrupt the game's own
//! delta arithmetic in a way that looks like a control-feel bug rather than a
//! modelling error.
//!
//! `TrackballEmu.v` is *not* evidence about the arcade hardware — it is the
//! MiSTer core's adapter turning a modern mouse or joystick into quadrature
//! pulses, complete with a sensitivity setting and a falloff ramp. Those are
//! host conveniences and are deliberately not reproduced.
//!
//! # No acceleration, and one presentation per frame
//!
//! Plan §6: raw deltas accumulate between frames and are presented **once per
//! frame**. [`Input::latch_frame`] does that fold, and the frame scheduler calls
//! it. Nothing applies pointer acceleration or smoothing — the plan calls this
//! out because acceleration silently changes game feel and is very hard to
//! diagnose after the fact.
//!
//! **UNVERIFIED:** the scaling between a host delta and one CPU-visible count.
//! The model treats one unit of [`Input::add_trackball_delta`] as one visible
//! count, bypassing the decoder's internal ÷2, because a host API supplies
//! movement rather than quadrature edges. Any feel-tuning belongs in the front
//! end, not here.

/// A switch the host can drive. Polarity is handled by the model — `true`
/// always means *pressed* or *active*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// Left start, which is also the left jump button (`STARTJMP1`, `MD6`).
    Start1,
    /// Right start, also the right jump button (`STARTJMP2`, `MD7`).
    Start2,
    /// Self-test (`MA.STS = MD4`).
    SelfTest,
    /// Slam/tilt.
    Slam,
    CoinAux,
    CoinLeft,
    CoinRight,
}

impl Switch {
    /// Bit position in the switch byte at `9600`.
    pub fn bit(self) -> u8 {
        match self {
            Switch::Start2 => 7,
            Switch::Start1 => 6,
            // bit 5 is VBLANK, not a switch
            Switch::SelfTest => 4,
            Switch::Slam => 3,
            Switch::CoinAux => 2,
            Switch::CoinLeft => 1,
            Switch::CoinRight => 0,
        }
    }
}

/// The VBLANK bit in the switch byte — a live video signal, not a switch.
pub const VBLANK_BIT: u8 = 5;

/// LETA register index for the vertical axis (`HW.TBV = 9400`).
pub const AXIS_VERTICAL: usize = 0;
/// LETA register index for the horizontal axis (`HW.TBH = 9401`).
pub const AXIS_HORIZONTAL: usize = 1;

pub struct Input {
    /// Which switches are currently pressed, one bit per [`Switch::bit`].
    /// Inversion happens on read, so this stays in "true = pressed" terms.
    pub pressed: u8,

    /// The four LETA axis counters as the CPU sees them. Axes 2 and 3 are
    /// unwired on this board (`CCastles.v:293` ties them off) and stay zero.
    pub counters: [u8; 4],

    /// Host movement accumulated since the last frame boundary, not yet
    /// visible to the CPU.
    pub pending_h: i32,
    pub pending_v: i32,

    /// Live VBLANK state, driven by the frame scheduler.
    pub vblank: bool,
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

impl Input {
    pub fn new() -> Self {
        Input {
            pressed: 0,
            counters: [0; 4],
            pending_h: 0,
            pending_v: 0,
            // A frame begins in vblank (lines 0-23), so this is the state at
            // cycle zero.
            vblank: true,
        }
    }

    /// Press or release a switch.
    pub fn set_switch(&mut self, switch: Switch, pressed: bool) {
        let mask = 1u8 << switch.bit();
        if pressed {
            self.pressed |= mask;
        } else {
            self.pressed &= !mask;
        }
    }

    pub fn switch_pressed(&self, switch: Switch) -> bool {
        self.pressed & (1 << switch.bit()) != 0
    }

    /// Accumulate host movement. Positive `dx` is right, positive `dy` is down.
    ///
    /// May be called any number of times between frames; the total is presented
    /// to the CPU once, at the next frame boundary. No acceleration is applied.
    pub fn add_trackball_delta(&mut self, dx: i32, dy: i32) {
        self.pending_h = self.pending_h.saturating_add(dx);
        self.pending_v = self.pending_v.saturating_add(dy);
    }

    /// Fold accumulated movement into the counters. Called once per frame.
    ///
    /// The counters wrap, exactly as the 9-bit hardware counter does. They are
    /// deliberately **not** clamped; see the module docs.
    pub fn latch_frame(&mut self) {
        self.counters[AXIS_HORIZONTAL] =
            (self.counters[AXIS_HORIZONTAL] as i32).wrapping_add(self.pending_h) as u8;
        self.counters[AXIS_VERTICAL] =
            (self.counters[AXIS_VERTICAL] as i32).wrapping_add(self.pending_v) as u8;
        self.pending_h = 0;
        self.pending_v = 0;
    }

    /// The switch byte at `9600-97FF`, in hardware polarity.
    pub fn switch_byte(&self) -> u8 {
        // Active low: start from all-ones, with VBLANK cleared because it is
        // the one bit that is *not* inverted, then clear what is pressed.
        let mut b = !(1u8 << VBLANK_BIT);
        b &= !self.pressed;
        if self.vblank {
            b |= 1 << VBLANK_BIT;
        }
        b
    }

    /// A LETA axis counter. `index` is `BA[1:0]`.
    pub fn leta_read(&self, index: usize) -> u8 {
        self.counters[index & 3]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_switch_byte_is_all_ones_except_vblank() {
        let mut i = Input::new();
        i.vblank = false;
        assert_eq!(i.switch_byte(), 0b1101_1111, "active low, VBLANK clear");
        i.vblank = true;
        assert_eq!(i.switch_byte(), 0xFF);
    }

    #[test]
    fn switches_read_active_low() {
        let mut i = Input::new();
        i.vblank = false;
        i.set_switch(Switch::CoinLeft, true);
        assert_eq!(i.switch_byte() & 0b10, 0, "pressed reads as zero");
        i.set_switch(Switch::CoinLeft, false);
        assert_eq!(i.switch_byte() & 0b10, 0b10);
    }

    #[test]
    fn the_bit_positions_match_the_rtl_and_the_game() {
        assert_eq!(Switch::Start2.bit(), 7);
        assert_eq!(Switch::Start1.bit(), 6);
        assert_eq!(VBLANK_BIT, 5, "MA.VBL = MD5");
        assert_eq!(Switch::SelfTest.bit(), 4, "MA.STS = MD4");
        assert_eq!(Switch::Slam.bit(), 3);
        // CG.MAC:336 — "COINS ARE IN D0-D2".
        assert_eq!(Switch::CoinAux.bit(), 2);
        assert_eq!(Switch::CoinLeft.bit(), 1);
        assert_eq!(Switch::CoinRight.bit(), 0);
    }

    #[test]
    fn vblank_is_not_inverted_unlike_every_other_bit() {
        // The one bit that is a live video signal rather than a switch.
        let mut i = Input::new();
        i.vblank = true;
        assert_ne!(i.switch_byte() & (1 << VBLANK_BIT), 0);
        i.vblank = false;
        assert_eq!(i.switch_byte() & (1 << VBLANK_BIT), 0);
    }

    #[test]
    fn deltas_are_presented_once_per_frame_not_continuously() {
        let mut i = Input::new();
        i.add_trackball_delta(5, 0);
        assert_eq!(
            i.leta_read(AXIS_HORIZONTAL),
            0,
            "movement must not be visible before the frame boundary"
        );
        i.latch_frame();
        assert_eq!(i.leta_read(AXIS_HORIZONTAL), 5);

        // A second latch with no new movement changes nothing.
        i.latch_frame();
        assert_eq!(i.leta_read(AXIS_HORIZONTAL), 5, "counter is absolute");
    }

    #[test]
    fn movement_within_a_frame_accumulates_into_one_presentation() {
        let mut i = Input::new();
        for _ in 0..10 {
            i.add_trackball_delta(3, -2);
        }
        assert_eq!(i.leta_read(AXIS_HORIZONTAL), 0);
        i.latch_frame();
        assert_eq!(i.leta_read(AXIS_HORIZONTAL), 30);
        assert_eq!(i.leta_read(AXIS_VERTICAL), (-20i32) as u8);
    }

    #[test]
    fn the_counter_wraps_and_is_never_saturated() {
        // SupportChips.v:297-307 is a plain 9-bit up/down counter. The game
        // differences it (CEN.MAC:318), so wrapping is invisible to it —
        // whereas a hardware clamp would corrupt that arithmetic.
        let mut i = Input::new();
        i.add_trackball_delta(250, 0);
        i.latch_frame();
        assert_eq!(i.leta_read(AXIS_HORIZONTAL), 250);
        i.add_trackball_delta(10, 0);
        i.latch_frame();
        assert_eq!(i.leta_read(AXIS_HORIZONTAL), 4, "250 + 10 wraps to 4");

        // And downwards through zero.
        let mut i = Input::new();
        i.add_trackball_delta(-1, 0);
        i.latch_frame();
        assert_eq!(i.leta_read(AXIS_HORIZONTAL), 255);
    }

    #[test]
    fn the_games_delta_arithmetic_survives_the_wrap() {
        // Replicates TR.DEL: read, subtract previous, store. The 8-bit
        // subtraction must recover the true movement across the wrap point.
        let mut i = Input::new();
        i.add_trackball_delta(250, 0);
        i.latch_frame();
        let prev = i.leta_read(AXIS_HORIZONTAL);

        i.add_trackball_delta(10, 0);
        i.latch_frame();
        let now = i.leta_read(AXIS_HORIZONTAL);

        let delta = now.wrapping_sub(prev); // SUB TR.I
        assert_eq!(delta, 10, "the wrap is invisible to the game");

        // Backwards, too: the delta reads as a negative 8-bit value.
        i.add_trackball_delta(-6, 0);
        i.latch_frame();
        let back = i.leta_read(AXIS_HORIZONTAL).wrapping_sub(now);
        assert_eq!(back as i8, -6);
    }

    #[test]
    fn the_axes_are_the_production_board_way_round() {
        // CG.MAC:5 sets CG.PC=1, so the production block at :120 is live:
        // HW.TBV = 9400 (register 0), HW.TBH = 9401 (register 1). The wirewrap
        // block at :62 has them swapped.
        assert_eq!(AXIS_VERTICAL, 0);
        assert_eq!(AXIS_HORIZONTAL, 1);
        let mut i = Input::new();
        i.add_trackball_delta(0, 7);
        i.latch_frame();
        assert_eq!(i.leta_read(0), 7, "9400 is vertical");
        assert_eq!(i.leta_read(1), 0, "9401 is horizontal");
    }

    #[test]
    fn the_unwired_axes_stay_at_zero() {
        // CCastles.v:293 ties X3/Y3/X4/Y4 off.
        let mut i = Input::new();
        i.add_trackball_delta(100, 100);
        i.latch_frame();
        assert_eq!(i.leta_read(2), 0);
        assert_eq!(i.leta_read(3), 0);
    }

    #[test]
    fn no_acceleration_is_applied() {
        // Ten single-unit moves and one ten-unit move must be identical. Any
        // acceleration or smoothing would separate them.
        let mut a = Input::new();
        for _ in 0..10 {
            a.add_trackball_delta(1, 1);
        }
        a.latch_frame();

        let mut b = Input::new();
        b.add_trackball_delta(10, 10);
        b.latch_frame();

        assert_eq!(a.counters, b.counters);
    }
}
