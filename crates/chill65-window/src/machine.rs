//! The machines, the controls, and the beam they produce.
//!
//! No GPU and no window here: this half turns key presses into switch states
//! and frames into beam-trace samples, and can be tested without either.
//!
//! # Two games, one path
//!
//! Space Duel and Tempest are different boards, and everything that differs
//! between them is named here rather than left to the caller: the frame
//! period, the deflection full scale, how a move's colour and intensity become
//! gun drive, which switches exist and what a key does to them. [`Game`] picks
//! the set, [`Controls`] and [`Producer`] carry it, and `live`, `capture` and
//! `window` are written against those and never against a particular game.
//!
//! # The conversion is transcribed, not reinvented
//!
//! Turning a frame's [`BeamMove`]s into samples follows
//! `chill65-runtime/tests/trace_sd.rs`, which is the reference and carries the
//! reasoning. The short version: the trace wants the beam's whole path, so
//! blanked travel appears as zero-drive samples rather than being skipped, and
//! each lit stroke is bracketed by Z edges so linear interpolation of drive
//! cannot bleed light into the dark. Positions are normalised per axis by the
//! generator's full scale, which `hardware.md` §14 derives from the board's own
//! diagnostic for Space Duel and `te::FULL_SCALE_X` from Tempest's.

use std::path::Path;

use beam_trace::{flags, Sample};
use chill65_runtime::sd::{run_frame_sd, SdInput, SdMachine};
use chill65_runtime::te::{run_frame_te, TeInput, TeMachine};
use chill65_runtime::vg::{intensity_drive, intensity_drive_color, BeamMove};
use chill65_runtime::Cpu;
use chill65_runtime::{sd, te, vg};

/// Generator ticks per second — how fast the beam sweeps.
///
/// **UNVERIFIED, fitted.** Nothing in either game's source times a vector;
/// `BeamMove::ticks` documents the tick model itself. `trace_sd.rs` measures
/// Space Duel's busiest attract frame at 32% of a frame period at this rate,
/// and the same rate is used for Tempest, whose generator is the same one — a
/// board difference would have to come from the analogue integrators, which
/// neither corpus describes.
pub const TICKS_PER_SECOND: f64 = 1_500_000.0;

/// Z-axis rise and fall, seconds.
///
/// **UNVERIFIED.** Blanking is not instant, and drive is interpolated linearly
/// between samples, so a stroke needs samples bracketing its edges tightly.
pub const Z_EDGE: f32 = 0.5e-6;

/// Which game a corpus holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Game {
    SpaceDuel,
    Tempest,
}

impl Game {
    /// Which game a source directory is, by a module only that game links.
    ///
    /// `AST2RD.MAC` and `ALEXEC.MAC` are each the first entry in their game's
    /// link order (`chill65_asm::sd`, `chill65_asm::te`), so a directory that
    /// can build at all has the right one and a directory that has both is not
    /// a corpus this frontend understands.
    pub fn detect(corpus: &Path) -> Result<Game, String> {
        let has = |name: &str| corpus.join(name).is_file();
        match (has("AST2RD.MAC"), has("ALEXEC.MAC")) {
            (true, false) => Ok(Game::SpaceDuel),
            (false, true) => Ok(Game::Tempest),
            (true, true) => Err(format!(
                "{} holds both AST2RD.MAC and ALEXEC.MAC, so it is two games \
                 at once — point CHILL65_CORPUS at one game's source",
                corpus.display()
            )),
            (false, false) => Err(format!(
                "{} holds neither AST2RD.MAC (Space Duel) nor ALEXEC.MAC \
                 (Tempest) — point CHILL65_CORPUS at a game's source directory",
                corpus.display()
            )),
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Game::SpaceDuel => "Space Duel",
            Game::Tempest => "Tempest",
        }
    }

    /// One frame of the machine, in seconds.
    ///
    /// Space Duel is 24576 cycles at 1.512 MHz, so 61.5234 Hz. Tempest is nine
    /// interrupts of 6,048 on the same clock, so 27.78 Hz — less than half,
    /// which the game itself rounds to "28 PER SECOND" (`ALWELG.MAC:580`). The
    /// real-time producer paces to this and the capture steps by it.
    pub fn frame_seconds(self) -> f64 {
        match self {
            Game::SpaceDuel => f64::from(sd::CYCLES_PER_FRAME) / f64::from(sd::CPU_HZ),
            Game::Tempest => f64::from(te::CYCLES_PER_FRAME) / f64::from(te::CPU_HZ),
        }
    }

    /// The refresh rate a trace header advertises.
    pub fn refresh_hz(self) -> f32 {
        (1.0 / self.frame_seconds()) as f32
    }

    /// The frames the machine runs before anything is worth exporting.
    ///
    /// Space Duel starts drawing around frame 99 and Tempest at frame 17
    /// (`boot_sd.rs`, `boot_te.rs`); both of these are well past that and past
    /// the power-on self-test behind it.
    pub fn warmup_frames(self) -> u32 {
        match self {
            Game::SpaceDuel => 300,
            Game::Tempest => 60,
        }
    }

    /// How long one requested coin holds the mech's line down.
    ///
    /// `COIN65.MAC` is byte-identical between the two corpora and accepts
    /// between 16 and 800 ms of coin present, rejecting anything longer as a
    /// stuck mechanism; `chill65-runtime/tests/credit_sd.rs` measures that as 2
    /// to 48 Space Duel frames and `hardware.md` §15.1 records it. Tempest's
    /// frames are 36 ms rather than 16, so the same milliseconds are far fewer
    /// frames — 1 to 22 — and the count has to be its own per game.
    pub fn coin_hold_frames(self) -> u32 {
        match self {
            Game::SpaceDuel => 20,
            Game::Tempest => 10,
        }
    }

    /// What the keys do, for printing at launch.
    pub fn bindings(self) -> &'static str {
        match self {
            Game::SpaceDuel => {
                "  5            insert coin        1      start
  2            game select        Space  fire
  Left/Right   rotate             Up     thrust
  LShift       shield             Esc    quit"
            }
            Game::Tempest => {
                "  5            insert coin        1      start, one player
  Left/Right   spinner            2      start, two players
  Space        fire               LShift superzapper
  Esc          quit"
            }
        }
    }
}

/// What the player is holding down, on either cabinet.
///
/// One value rather than two type parameters, because the window and the ring
/// buffer that carry it do not care which game is running and should not have
/// to be written twice to prove it. A `Controls` is always made by
/// [`Controls::new`] from the same [`Game`] the [`Producer`] was, so the two
/// cannot disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Controls {
    Sd(SdControls),
    Te(TeControls),
}

impl Controls {
    pub fn new(game: Game) -> Self {
        match game {
            Game::SpaceDuel => Controls::Sd(SdControls::default()),
            Game::Tempest => Controls::Te(TeControls::default()),
        }
    }

    /// Coins the player has asked for since the machine started.
    pub fn coin_pulses(&self) -> u64 {
        match self {
            Controls::Sd(c) => c.coin_pulses,
            Controls::Te(c) => c.coin_pulses,
        }
    }

    /// Ask for one more coin.
    pub fn insert_coin(&mut self) {
        match self {
            Controls::Sd(c) => c.coin_pulses += 1,
            Controls::Te(c) => c.coin_pulses += 1,
        }
    }
}

/// Space Duel's panel.
///
/// `coin_pulses` counts *requested insertions* rather than holding a line,
/// because a coin needs about 53 frames of debounce (`play_sd.rs` derives that
/// from `COIN65.MAC`) and a key tap is far shorter. The producer latches the
/// line for long enough on the player's behalf, so a quick tap still registers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SdControls {
    pub start: bool,
    pub fire: bool,
    pub thrust: bool,
    pub rotate_left: bool,
    pub rotate_right: bool,
    pub shield: bool,
    pub game_select: bool,
    pub coin_pulses: u64,
}

impl SdControls {
    /// The switch state this implies, on an upright cabinet.
    pub fn to_input(self) -> SdInput {
        SdInput {
            start: self.start,
            fire: self.fire,
            thrust: self.thrust,
            rotate_left: self.rotate_left,
            rotate_right: self.rotate_right,
            shield: self.shield,
            game_select: self.game_select,
            ..SdInput::upright()
        }
    }
}

/// Tempest's panel.
///
/// The spinner is **not** here. `TeInput::spinner` is an absolute four-bit
/// counter that the game differences against last frame's
/// (`ALHARD.MAC:63-81`), so it is a position rather than a switch, and a
/// keyboard offers no position — only "turning, this way". The direction keys
/// live here and [`TeProducer`] does the turning, one frame at a time, because
/// the frame boundary is where a spinner count belongs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TeControls {
    pub start1: bool,
    pub start2: bool,
    pub fire: bool,
    pub superzapper: bool,
    pub rotate_left: bool,
    pub rotate_right: bool,
    pub coin_pulses: u64,
}

/// Apply a key to the controls. Returns whether the key was one of ours.
///
/// Coin is press-only: a release must not undo an insertion, and a held key
/// must not queue a second one — the window filters auto-repeat before calling
/// this.
pub fn apply_key(controls: &mut Controls, key: winit::keyboard::KeyCode, pressed: bool) -> bool {
    match controls {
        Controls::Sd(c) => apply_key_sd(c, key, pressed),
        Controls::Te(c) => apply_key_te(c, key, pressed),
    }
}

pub fn apply_key_sd(sd: &mut SdControls, key: winit::keyboard::KeyCode, pressed: bool) -> bool {
    use winit::keyboard::KeyCode;
    match key {
        KeyCode::Digit5 => {
            if pressed {
                sd.coin_pulses += 1;
            }
        }
        KeyCode::Digit1 => sd.start = pressed,
        KeyCode::Digit2 => sd.game_select = pressed,
        KeyCode::ArrowLeft => sd.rotate_left = pressed,
        KeyCode::ArrowRight => sd.rotate_right = pressed,
        KeyCode::ArrowUp => sd.thrust = pressed,
        KeyCode::Space => sd.fire = pressed,
        KeyCode::ShiftLeft => sd.shield = pressed,
        _ => return false,
    }
    true
}

pub fn apply_key_te(te: &mut TeControls, key: winit::keyboard::KeyCode, pressed: bool) -> bool {
    use winit::keyboard::KeyCode;
    match key {
        KeyCode::Digit5 => {
            if pressed {
                te.coin_pulses += 1;
            }
        }
        KeyCode::Digit1 => te.start1 = pressed,
        KeyCode::Digit2 => te.start2 = pressed,
        KeyCode::ArrowLeft => te.rotate_left = pressed,
        KeyCode::ArrowRight => te.rotate_right = pressed,
        KeyCode::Space => te.fire = pressed,
        KeyCode::ShiftLeft => te.superzapper = pressed,
        _ => return false,
    }
    true
}

/// A machine that hands back beam samples, one frame at a time.
pub enum Producer {
    Sd(SdProducer),
    Te(TeProducer),
}

impl Producer {
    /// Assemble the ROMs from a source directory and start the machine.
    ///
    /// This is the slow part — the link takes a couple of seconds — so callers
    /// that care about responsiveness should do it off the main thread.
    pub fn from_corpus(game: Game, corpus: &Path) -> Result<Self, String> {
        match game {
            Game::SpaceDuel => SdProducer::from_corpus(corpus).map(Producer::Sd),
            Game::Tempest => TeProducer::from_corpus(corpus).map(Producer::Te),
        }
    }

    pub fn game(&self) -> Game {
        match self {
            Producer::Sd(_) => Game::SpaceDuel,
            Producer::Te(_) => Game::Tempest,
        }
    }

    /// Run the warm-up frames and throw the beam away.
    pub fn warm_up(&mut self) -> Result<(), String> {
        let idle = Controls::new(self.game());
        for _ in 0..self.game().warmup_frames() {
            self.step_frame(&idle, false)?;
        }
        Ok(())
    }

    /// Run one frame with the given switches, and return the beam it drew.
    ///
    /// `coin` is the left mechanism's line, held or not; the caller owns the
    /// timing of that because the debounce is longer than a key press.
    /// Timestamps start at zero for the frame; the caller places them on its
    /// own clock.
    pub fn step_frame(&mut self, controls: &Controls, coin: bool) -> Result<Vec<Sample>, String> {
        match (self, controls) {
            (Producer::Sd(p), Controls::Sd(c)) => p.step_frame(*c, coin),
            (Producer::Te(p), Controls::Te(c)) => p.step_frame(*c, coin),
            _ => unreachable!("controls and machine are made from the same Game"),
        }
    }

    /// The picture's fingerprint, for comparing two machines.
    pub fn frame_hash(&self) -> u64 {
        match self {
            Producer::Sd(p) => p.machine.frame_hash(),
            Producer::Te(p) => p.machine.frame_hash(),
        }
    }

    /// Strokes drawn in the last frame.
    pub fn segments(&self) -> usize {
        match self {
            Producer::Sd(p) => p.machine.segments.len(),
            Producer::Te(p) => p.machine.segments.len(),
        }
    }
}

/// The left coin, `0800` D2 on Space Duel. A set bit means a coin is present.
const SD_COIN_BIT: u8 = 0b100;

/// The left coin, `IN1` D2 on Tempest (`ALCOMN.MAC:243-251`). `TeInput::coins`
/// is active high and `te.rs` inverts it on the way to the port.
const TE_COIN_BIT: u8 = 0b100;

pub struct SdProducer {
    cpu: Cpu,
    machine: SdMachine,
}

impl SdProducer {
    pub fn from_corpus(corpus: &Path) -> Result<Self, String> {
        let images = chill65_asm::sd::images(corpus)?;
        let mut machine = SdMachine::new();
        machine
            .load_roms(&images.ship, &images.program)
            .map_err(|e| format!("loading the images: {e}"))?;
        machine.set_options(0, 0);
        machine.self_test = false;
        machine.trace_beam = true;
        machine.input = SdInput::upright();

        let mut cpu = Cpu::new();
        cpu.reset(&mut machine);
        Ok(SdProducer { cpu, machine })
    }

    pub fn step_frame(&mut self, controls: SdControls, coin: bool) -> Result<Vec<Sample>, String> {
        self.machine.input = controls.to_input();
        self.machine.coins = if coin { SD_COIN_BIT } else { 0 };
        run_frame_sd(&mut self.cpu, &mut self.machine).map_err(|e| format!("{e:?}"))?;
        Ok(samples_of(
            &self.machine.beam,
            (vg::FULL_SCALE_X, vg::FULL_SCALE_Y),
            drive_of_sd,
        ))
    }
}

/// How many counts the spinner turns per frame while a direction is held.
///
/// **UNVERIFIED.** Nothing says how many counts the real control gives per
/// revolution, so this is a playable rate rather than a measured one: one count
/// a frame is 27.8 a second, and the pot has sixteen positions, so the claw
/// crosses the web in a little under two seconds. The upper bound is not a
/// matter of taste — `ALHARD.MAC:71-76` reads the difference modulo sixteen and
/// calls anything from 8 upward negative, so a step of 8 or more would turn the
/// wrong way.
const SPINNER_STEP: u8 = 1;

pub struct TeProducer {
    cpu: Cpu,
    machine: TeMachine,
    /// The spinner's absolute position, 0-15, which the producer turns.
    spinner: u8,
}

impl TeProducer {
    pub fn from_corpus(corpus: &Path) -> Result<Self, String> {
        let images = chill65_asm::te::images(corpus)?;
        let mut machine = TeMachine::new();
        machine
            .load_roms(&images.vec_rom, &images.program)
            .map_err(|e| format!("loading the images: {e}"))?;
        machine.trace_beam = true;
        // `TeInput::upright` clears the test switch, which this board reads
        // active low, so the machine takes its normal boot. It also leaves both
        // option bytes at zero, which `ALEXEC.MAC:244-248` reads as free play
        // and grants two credits for — so the start button works with no coin,
        // and the coin slot still does what it should when one is used.
        machine.input = TeInput::upright();

        let mut cpu = Cpu::new();
        cpu.reset(&mut machine);
        Ok(TeProducer {
            cpu,
            machine,
            spinner: 0,
        })
    }

    pub fn step_frame(&mut self, controls: TeControls, coin: bool) -> Result<Vec<Sample>, String> {
        if controls.rotate_left {
            self.spinner = self.spinner.wrapping_sub(SPINNER_STEP) & 0x0F;
        }
        if controls.rotate_right {
            self.spinner = self.spinner.wrapping_add(SPINNER_STEP) & 0x0F;
        }
        self.machine.input = TeInput {
            spinner: self.spinner,
            fire: controls.fire,
            superzapper: controls.superzapper,
            start1: controls.start1,
            start2: controls.start2,
            coins: if coin { TE_COIN_BIT } else { 0 },
            ..TeInput::upright()
        };
        run_frame_te(&mut self.cpu, &mut self.machine).map_err(|e| format!("{e:?}"))?;

        // Copied out because the closure below and the beam are both borrows of
        // the machine, and because it is the palette *this* frame drew under.
        let palette = self.machine.color_ram;
        Ok(samples_of(
            &self.machine.beam,
            (te::FULL_SCALE_X, te::FULL_SCALE_Y),
            move |mv| drive_of_te(mv, &palette),
        ))
    }
}

/// A lit move's drive on Space Duel: its colour's channels scaled by its
/// intensity.
///
/// Colour is three bits — bit 0 blue, bit 1 green, bit 2 red
/// (`AST2RD.MAC:244-251`).
fn drive_of_sd(mv: &BeamMove) -> [f32; 3] {
    let level = intensity_drive(mv.intensity);
    [
        ((mv.color >> 2) & 1) as f32 * level,
        ((mv.color >> 1) & 1) as f32 * level,
        (mv.color & 1) as f32 * level,
    ]
}

/// The same on Tempest, where the colour is an index into a palette the game
/// rewrites every frame.
fn drive_of_te(mv: &BeamMove, palette: &[u8; 16]) -> [f32; 3] {
    let level = intensity_drive_color(mv.intensity);
    let rgb = palette_rgb(palette[(mv.color & 0x0F) as usize]);
    [rgb[0] * level, rgb[1] * level, rgb[2] * level]
}

/// One colour-RAM entry as gun drive, before intensity.
///
/// The entries are four bits and **active low**: `ALCOMN.MAC:371-382` builds
/// every colour the game has by ANDing three masks together, so a *clear* bit
/// lights a gun and `ZBLACK=0F` is nothing lit at all.
///
/// ```text
/// FRED=0C  FBLUE=0B  FGREEN=07          ZBLACK=0F
/// ZWHITE=FRED&FBLUE&FGREEN  ZYELLO=FRED&FGREEN  ZPURPL=FRED&FBLUE
/// ZTURQOI=FGREEN&FBLUE      ZRED=FRED  ZGREEN=FGREEN  ZBLUE=FBLUE
/// ```
///
/// Solving those eight for the bit assignment gives green in bit 3, blue in
/// bit 2 and red in bits 1-0, and the assignment is over-determined — every one
/// of the eight has to come out right, and does.
///
/// **UNVERIFIED, and it does not matter here: what red's second bit is.**
/// `HRED=0D` clears bit 1 and leaves bit 0 set, so red is either a two-bit gun
/// at two-thirds or a one-bit gun with the brightness bit off. Nothing in
/// either corpus distinguishes them, and nothing needs to: `HRED` is defined
/// and never used, and the sixteen entries the game actually loads
/// (`ALDIS2.MAC:2395-2402` through `COLTAB`, read back off the running machine
/// as `00 04 08 0C 03 07 0B 0B 0B 00 00 00 0C 00 00 00`) are the eight named
/// colours and nothing else. In every one of them red is full or absent.
fn palette_rgb(entry: u8) -> [f32; 3] {
    // Active low, so invert first and read the guns out of the ones.
    let on = !entry;
    [
        (on & 0b11) as f32 / 3.0,
        ((on >> 3) & 1) as f32,
        ((on >> 2) & 1) as f32,
    ]
}

/// One frame's beam as samples, timestamped from zero.
///
/// `scale` is the generator's full-scale deflection per axis, and `drive` is
/// what one lit move does to the three guns — the two things that differ
/// between the boards.
///
/// The frame opens with a discontinuity at the centre because the GO strobe
/// resets the beam there, so nothing is deposited across the gap from wherever
/// the previous frame left the spot.
pub fn samples_of(
    beam: &[BeamMove],
    scale: (i32, i32),
    drive: impl Fn(&BeamMove) -> [f32; 3],
) -> Vec<Sample> {
    // Normalised deflection. Per axis, because a trace does not encode aspect
    // ratio — the tube profile supplies the 4:3.
    let norm_x = |v: i32| v as f32 / scale.0 as f32;
    let norm_y = |v: i32| v as f32 / scale.1 as f32;

    let mut out = Vec::with_capacity(beam.len() * 3 + 1);
    let mut t = Z_EDGE;
    out.push(sample(0.0, 0.0, [0.0; 3], t, flags::DISCONTINUITY));

    for mv in beam {
        if mv.jump {
            t += Z_EDGE;
            out.push(sample(
                norm_x(mv.x1),
                norm_y(mv.y1),
                [0.0; 3],
                t,
                flags::DISCONTINUITY,
            ));
            continue;
        }

        let dt = ((mv.ticks() as f64 / TICKS_PER_SECOND) as f32).max(Z_EDGE);
        if mv.intensity == 0 {
            // Blanked travel: real movement, no light, and it takes time.
            t += dt;
            out.push(sample(norm_x(mv.x1), norm_y(mv.y1), [0.0; 3], t, 0));
            continue;
        }

        // Z on where the stroke starts, sweep, Z off where it ends. The sample
        // before this is always zero-drive at the same point, so the rise is a
        // fixed-position edge rather than a lit smear.
        let d = drive(mv);
        t += Z_EDGE;
        out.push(sample(norm_x(mv.x0), norm_y(mv.y0), d, t, 0));
        t += dt;
        out.push(sample(norm_x(mv.x1), norm_y(mv.y1), d, t, 0));
        t += Z_EDGE;
        out.push(sample(norm_x(mv.x1), norm_y(mv.y1), [0.0; 3], t, 0));
    }
    out
}

fn sample(x: f32, y: f32, drive: [f32; 3], t: f32, flags: u32) -> Sample {
    Sample {
        x,
        y,
        drive_r: drive[0],
        drive_g: drive[1],
        drive_b: drive[2],
        t,
        flags,
        reserved: 0,
    }
}

/// Place a frame's samples on a running clock, keeping `t` strictly
/// increasing.
///
/// Frames abut, so the first sample of one can land on the last of the
/// previous; the format requires strict increase, and the duplicate carries no
/// new information. Nudging by one float ulp is what `tube-shell`'s own
/// producer does.
pub fn offset_onto(samples: &mut [Sample], origin: f64, last_t: &mut f32) {
    for s in samples {
        s.t += origin as f32;
        if s.t <= *last_t {
            s.t = f32::from_bits(last_t.to_bits() + 1);
        }
        *last_t = s.t;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    /// The full-scale pair for a game, for the tests that convert by hand.
    fn scale(game: Game) -> (i32, i32) {
        match game {
            Game::SpaceDuel => (vg::FULL_SCALE_X, vg::FULL_SCALE_Y),
            Game::Tempest => (te::FULL_SCALE_X, te::FULL_SCALE_Y),
        }
    }

    #[test]
    fn every_space_duel_binding_sets_and_clears_its_switch() {
        let cases: [(KeyCode, fn(&SdControls) -> bool); 7] = [
            (KeyCode::Digit1, |c| c.start),
            (KeyCode::Digit2, |c| c.game_select),
            (KeyCode::ArrowLeft, |c| c.rotate_left),
            (KeyCode::ArrowRight, |c| c.rotate_right),
            (KeyCode::ArrowUp, |c| c.thrust),
            (KeyCode::Space, |c| c.fire),
            (KeyCode::ShiftLeft, |c| c.shield),
        ];
        for (key, read) in cases {
            let mut c = SdControls::default();
            assert!(apply_key_sd(&mut c, key, true), "{key:?} is one of ours");
            assert!(read(&c), "{key:?} pressed");
            assert!(apply_key_sd(&mut c, key, false));
            assert!(!read(&c), "{key:?} released");
        }
    }

    #[test]
    fn every_tempest_binding_sets_and_clears_its_switch() {
        let cases: [(KeyCode, fn(&TeControls) -> bool); 6] = [
            (KeyCode::Digit1, |c| c.start1),
            (KeyCode::Digit2, |c| c.start2),
            (KeyCode::ArrowLeft, |c| c.rotate_left),
            (KeyCode::ArrowRight, |c| c.rotate_right),
            (KeyCode::Space, |c| c.fire),
            (KeyCode::ShiftLeft, |c| c.superzapper),
        ];
        for (key, read) in cases {
            let mut c = TeControls::default();
            assert!(apply_key_te(&mut c, key, true), "{key:?} is one of ours");
            assert!(read(&c), "{key:?} pressed");
            assert!(apply_key_te(&mut c, key, false));
            assert!(!read(&c), "{key:?} released");
        }
    }

    /// Tempest has no thrust pedal, and pressing one must not silently arm
    /// something else.
    #[test]
    fn a_key_belonging_to_the_other_game_is_refused() {
        let mut te = Controls::new(Game::Tempest);
        assert!(!apply_key(&mut te, KeyCode::ArrowUp, true), "no thrust");
        assert_eq!(te, Controls::new(Game::Tempest));

        let mut sd = Controls::new(Game::SpaceDuel);
        assert!(!apply_key(&mut sd, KeyCode::KeyQ, true));
        assert_eq!(sd, Controls::new(Game::SpaceDuel));
    }

    #[test]
    fn a_coin_counts_on_press_only_in_either_game() {
        for game in [Game::SpaceDuel, Game::Tempest] {
            let mut c = Controls::new(game);
            apply_key(&mut c, KeyCode::Digit5, true);
            apply_key(&mut c, KeyCode::Digit5, false);
            apply_key(&mut c, KeyCode::Digit5, true);
            apply_key(&mut c, KeyCode::Digit5, false);
            assert_eq!(c.coin_pulses(), 2, "{game:?}: two taps, two coins");
        }
    }

    #[test]
    fn the_controls_become_switches_on_an_upright_cabinet() {
        let mut c = SdControls::default();
        c.thrust = true;
        c.fire = true;
        let input = c.to_input();
        assert!(input.thrust && input.fire);
        assert!(input.upright, "the cabinet stays upright");
        assert!(!input.shield && !input.start);
    }

    #[test]
    fn a_frame_opens_at_the_centre_with_a_discontinuity() {
        let out = samples_of(&[], scale(Game::SpaceDuel), drive_of_sd);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].x, out[0].y), (0.0, 0.0));
        assert_eq!(out[0].flags, flags::DISCONTINUITY);
        assert_eq!(out[0].drive_r, 0.0);
    }

    #[test]
    fn a_lit_move_is_bracketed_and_a_blanked_one_is_not() {
        let lit = BeamMove {
            x0: 0,
            y0: 0,
            x1: 128,
            y1: 96,
            intensity: 7,
            color: 4, // red
            jump: false,
        };
        let blanked = BeamMove {
            intensity: 0,
            ..lit
        };

        let out = samples_of(&[lit], scale(Game::SpaceDuel), drive_of_sd);
        assert_eq!(out.len(), 4, "open, Z on, sweep, Z off");
        assert_eq!(out[1].drive_r, 1.0, "code 7 is full drive");
        assert_eq!(out[1].drive_g, 0.0, "colour 4 is red alone");
        assert_eq!((out[2].x, out[2].y), (0.5, 0.5), "128/256 and 96/192");
        assert_eq!(out[3].drive_r, 0.0, "blanked again at the end");

        let out = samples_of(&[blanked], scale(Game::SpaceDuel), drive_of_sd);
        assert_eq!(out.len(), 2, "the open sample and the travel");
        assert_eq!(out[1].drive_r, 0.0);
        assert!(out[1].t > out[0].t, "travel takes time");
    }

    #[test]
    fn a_centre_is_a_discontinuity_that_costs_no_time() {
        let jump = BeamMove {
            x0: 100,
            y0: 50,
            x1: 0,
            y1: 0,
            intensity: 0,
            color: 0,
            jump: true,
        };
        let out = samples_of(&[jump], scale(Game::SpaceDuel), drive_of_sd);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].flags, flags::DISCONTINUITY);
        assert_eq!((out[1].x, out[1].y), (0.0, 0.0));
    }

    /// The eight colours `ALCOMN.MAC:375-382` names, each read back as the
    /// guns it is supposed to light.
    #[test]
    fn the_palette_decodes_every_colour_the_game_names() {
        let cases = [
            (0x00, [1.0, 1.0, 1.0], "ZWHITE"),
            (0x04, [1.0, 1.0, 0.0], "ZYELLO"),
            (0x08, [1.0, 0.0, 1.0], "ZPURPL"),
            (0x0C, [1.0, 0.0, 0.0], "ZRED"),
            (0x03, [0.0, 1.0, 1.0], "ZTURQOI"),
            (0x07, [0.0, 1.0, 0.0], "ZGREEN"),
            (0x0B, [0.0, 0.0, 1.0], "ZBLUE"),
            (0x0F, [0.0, 0.0, 0.0], "ZBLACK"),
        ];
        for (entry, want, name) in cases {
            assert_eq!(palette_rgb(entry), want, "{name} = {entry:02X}");
        }
    }

    /// Tempest's move carries an index, not a colour, and the palette in force
    /// decides what it means.
    #[test]
    fn a_tempest_move_is_coloured_by_the_palette_and_dimmed_by_its_code() {
        // The palette the machine holds after boot; index 3 is ZRED.
        let palette = [
            0x00, 0x04, 0x08, 0x0C, 0x03, 0x07, 0x0B, 0x0B, 0x0B, 0x00, 0x00, 0x00, 0x0C, 0x00,
            0x00, 0x00,
        ];
        let mv = BeamMove {
            x0: 0,
            y0: 0,
            x1: 250,
            y1: 270,
            intensity: 12,
            color: 3,
            jump: false,
        };
        let drive = drive_of_te(&mv, &palette);
        assert!(drive[0] > 0.0, "index 3 is ZRED, so red is lit");
        assert_eq!([drive[1], drive[2]], [0.0, 0.0], "and nothing else is");

        // A ZZ code of 6 is the same VGBRIT as a status nibble of 12.
        let dimmer = drive_of_te(&BeamMove { intensity: 6, ..mv }, &palette);
        assert_eq!(drive, dimmer);

        let out = samples_of(&[mv], scale(Game::Tempest), |m| drive_of_te(m, &palette));
        assert_eq!(
            (out[2].x, out[2].y),
            (1.0, 1.0),
            "the boundary box's corner is the corner of the tube"
        );
    }

    #[test]
    fn each_board_keeps_its_own_frame_rate() {
        // 24576 cycles at 1.512 MHz against nine intervals of 6048.
        assert!((Game::SpaceDuel.refresh_hz() - 61.5234).abs() < 0.001);
        assert!(
            (Game::Tempest.refresh_hz() - 27.78).abs() < 0.01,
            "the game's own comment rounds this to 28 per second"
        );
    }

    /// `ALHARD.MAC:71-76` reads the spinner's difference modulo sixteen and
    /// treats 8 and up as negative, so a step at or past that turns backwards.
    #[test]
    fn the_spinner_step_cannot_be_mistaken_for_the_other_direction() {
        assert!(SPINNER_STEP > 0 && SPINNER_STEP < 8);
    }

    /// A corpus is one game or the frontend says which two it looks like.
    #[test]
    fn the_corpus_names_its_own_game() {
        let Ok(corpus) = std::env::var("CHILL65_CORPUS") else {
            eprintln!("CHILL65_CORPUS unset — skipping the detection test");
            return;
        };
        let game = Game::detect(Path::new(&corpus)).expect("a game");
        println!("{} is {}", corpus, game.title());

        let nowhere = Path::new("/nonexistent-corpus");
        assert!(Game::detect(nowhere).is_err());
    }

    /// The real machine's beam, converted and validated.
    ///
    /// Gated on the corpus by an environment check rather than `#[ignore]`, so
    /// that a plain `cargo test` stays meaningful here and a `cargo test` with
    /// `CHILL65_CORPUS` set exercises the whole path without extra flags.
    #[test]
    fn real_frames_convert_to_a_trace_the_renderer_would_accept() {
        let Ok(corpus) = std::env::var("CHILL65_CORPUS") else {
            eprintln!("CHILL65_CORPUS unset — skipping the conversion test");
            return;
        };
        let corpus = Path::new(&corpus);
        let game = Game::detect(corpus).expect("a game");

        let mut producer = Producer::from_corpus(game, corpus).expect("build from corpus");
        producer.warm_up().expect("warm up");

        let idle = Controls::new(game);
        let mut all = Vec::new();
        let mut last = 0.0f32;
        let mut opened = 0;
        for frame in 0..5u32 {
            let mut s = producer.step_frame(&idle, false).expect("a frame");
            assert!(s.len() > 1, "frame {frame} drew nothing at all");
            assert_eq!(
                s[0].flags,
                flags::DISCONTINUITY,
                "frame {frame} must open with the GO strobe's recentre"
            );
            opened += 1;
            offset_onto(&mut s, f64::from(frame) * game.frame_seconds(), &mut last);
            all.extend(s);
        }

        assert_eq!(opened, 5);
        assert!(
            all.iter().any(|s| s.drive_r + s.drive_g + s.drive_b > 0.0),
            "five frames of attract and nothing was lit"
        );
        assert!(
            all.iter()
                .any(|s| s.drive_r + s.drive_g + s.drive_b == 0.0),
            "no blanked travel — the beam appears to teleport"
        );
        beam_trace::validate(&all).expect("the renderer's own validator");
        println!(
            "{}: {} samples over {:.4} s",
            game.title(),
            all.len(),
            last
        );
    }

    #[test]
    fn offsetting_keeps_time_strictly_increasing_across_frames() {
        let mut last = 0.0f32;
        let mut all = Vec::new();
        for frame in 0..3 {
            let mut s = samples_of(
                &[BeamMove {
                    x0: 0,
                    y0: 0,
                    x1: 64,
                    y1: 0,
                    intensity: 6,
                    color: 7,
                    jump: false,
                }],
                scale(Game::SpaceDuel),
                drive_of_sd,
            );
            offset_onto(
                &mut s,
                frame as f64 * Game::SpaceDuel.frame_seconds(),
                &mut last,
            );
            all.extend(s);
        }
        for pair in all.windows(2) {
            assert!(pair[1].t > pair[0].t, "{} follows {}", pair[1].t, pair[0].t);
        }
        beam_trace::validate(&all).expect("a valid trace");
    }
}
