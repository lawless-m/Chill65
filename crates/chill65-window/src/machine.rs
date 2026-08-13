//! The machine, the controls, and the beam it produces.
//!
//! No GPU and no window here: this half turns key presses into switch states
//! and frames into beam-trace samples, and can be tested without either.
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
//! diagnostic.

use std::path::Path;

use beam_trace::{flags, Sample};
use chill65_runtime::sd::{run_frame_sd, SdInput, SdMachine, CPU_HZ, CYCLES_PER_FRAME};
use chill65_runtime::vg::{intensity_drive, BeamMove, FULL_SCALE_X, FULL_SCALE_Y};
use chill65_runtime::Cpu;

/// One frame of the machine, in seconds: 24576 cycles at 1.512 MHz, so
/// 61.5234 Hz. The real-time producer paces to this and the capture steps by
/// it.
pub const FRAME_SECONDS: f64 = CYCLES_PER_FRAME as f64 / CPU_HZ as f64;

/// The refresh rate a trace header advertises.
pub const REFRESH_HZ: f32 = CPU_HZ as f32 / CYCLES_PER_FRAME as f32;

/// Generator ticks per second — how fast the beam sweeps.
///
/// **UNVERIFIED, fitted.** Nothing in the game source times a vector;
/// `BeamMove::ticks` documents the tick model itself. `trace_sd.rs` measures
/// the busiest attract frame at 32% of a frame period at this rate.
pub const TICKS_PER_SECOND: f64 = 1_500_000.0;

/// Z-axis rise and fall, seconds.
///
/// **UNVERIFIED.** Blanking is not instant, and drive is interpolated linearly
/// between samples, so a stroke needs samples bracketing its edges tightly.
pub const Z_EDGE: f32 = 0.5e-6;

/// The frames the machine runs before anything is worth exporting. Drawing
/// starts around frame 99 (`boot_sd.rs`); this is well past the self-test.
pub const WARMUP_FRAMES: u32 = 300;

/// What the player is holding down.
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

/// Apply a key to the controls. Returns whether the key was one of ours.
///
/// Coin is press-only: a release must not undo an insertion, and a held key
/// must not queue a second one — the window filters auto-repeat before calling
/// this.
pub fn apply_key(sd: &mut SdControls, key: winit::keyboard::KeyCode, pressed: bool) -> bool {
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

/// What the keys do, for printing at launch.
pub const BINDINGS: &str = "\
  5            insert coin        1      start
  2            game select        Space  fire
  Left/Right   rotate             Up     thrust
  LShift       shield             Esc    quit";

/// A Space Duel machine that hands back beam samples, one frame at a time.
pub struct SdProducer {
    cpu: Cpu,
    machine: SdMachine,
}

impl SdProducer {
    /// Assemble the ROMs from a source directory and start the machine.
    ///
    /// This is the slow part — the fifteen-module link takes a couple of
    /// seconds — so callers that care about responsiveness should do it off
    /// the main thread.
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

    /// Run [`WARMUP_FRAMES`] frames and throw the beam away.
    pub fn warm_up(&mut self) -> Result<(), String> {
        for _ in 0..WARMUP_FRAMES {
            run_frame_sd(&mut self.cpu, &mut self.machine).map_err(|e| format!("{e:?}"))?;
        }
        Ok(())
    }

    /// Run one frame with the given switches, and return the beam it drew.
    ///
    /// Timestamps start at zero for the frame; the caller places them on its
    /// own clock.
    pub fn step_frame(&mut self, input: SdInput, coins: u8) -> Result<Vec<Sample>, String> {
        self.machine.input = input;
        self.machine.coins = coins;
        run_frame_sd(&mut self.cpu, &mut self.machine).map_err(|e| format!("{e:?}"))?;
        Ok(samples_of(&self.machine.beam))
    }

    /// The picture's fingerprint, for comparing two machines.
    pub fn frame_hash(&self) -> u64 {
        self.machine.frame_hash()
    }

    /// Strokes drawn in the last frame.
    pub fn segments(&self) -> usize {
        self.machine.segments.len()
    }
}

/// Normalised deflection. Per axis, because a trace does not encode aspect
/// ratio — the tube profile supplies the 4:3.
fn norm_x(v: i32) -> f32 {
    v as f32 / FULL_SCALE_X as f32
}

fn norm_y(v: i32) -> f32 {
    v as f32 / FULL_SCALE_Y as f32
}

/// A lit move's drive: its colour's channels scaled by its intensity.
///
/// Colour is three bits — bit 0 blue, bit 1 green, bit 2 red
/// (`AST2RD.MAC:244-251`).
fn drive_of(mv: &BeamMove) -> [f32; 3] {
    let level = intensity_drive(mv.intensity);
    [
        ((mv.color >> 2) & 1) as f32 * level,
        ((mv.color >> 1) & 1) as f32 * level,
        (mv.color & 1) as f32 * level,
    ]
}

/// One frame's beam as samples, timestamped from zero.
///
/// The frame opens with a discontinuity at the centre because the GO strobe
/// resets the beam there, so nothing is deposited across the gap from wherever
/// the previous frame left the spot.
pub fn samples_of(beam: &[BeamMove]) -> Vec<Sample> {
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
        let drive = drive_of(mv);
        t += Z_EDGE;
        out.push(sample(norm_x(mv.x0), norm_y(mv.y0), drive, t, 0));
        t += dt;
        out.push(sample(norm_x(mv.x1), norm_y(mv.y1), drive, t, 0));
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

    #[test]
    fn every_binding_sets_and_clears_its_switch() {
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
            assert!(apply_key(&mut c, key, true), "{key:?} is one of ours");
            assert!(read(&c), "{key:?} pressed");
            assert!(apply_key(&mut c, key, false));
            assert!(!read(&c), "{key:?} released");
        }
    }

    #[test]
    fn a_coin_counts_on_press_only() {
        let mut c = SdControls::default();
        apply_key(&mut c, KeyCode::Digit5, true);
        apply_key(&mut c, KeyCode::Digit5, false);
        apply_key(&mut c, KeyCode::Digit5, true);
        apply_key(&mut c, KeyCode::Digit5, false);
        assert_eq!(c.coin_pulses, 2, "two taps, two coins");
    }

    #[test]
    fn an_unmapped_key_is_refused_and_changes_nothing() {
        let mut c = SdControls::default();
        assert!(!apply_key(&mut c, KeyCode::KeyQ, true));
        assert_eq!(c, SdControls::default());
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
        let out = samples_of(&[]);
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

        let out = samples_of(&[lit]);
        assert_eq!(out.len(), 4, "open, Z on, sweep, Z off");
        assert_eq!(out[1].drive_r, 1.0, "code 7 is full drive");
        assert_eq!(out[1].drive_g, 0.0, "colour 4 is red alone");
        assert_eq!((out[2].x, out[2].y), (0.5, 0.5), "128/256 and 96/192");
        assert_eq!(out[3].drive_r, 0.0, "blanked again at the end");

        let out = samples_of(&[blanked]);
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
        let out = samples_of(&[jump]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].flags, flags::DISCONTINUITY);
        assert_eq!((out[1].x, out[1].y), (0.0, 0.0));
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

        let mut producer =
            SdProducer::from_corpus(std::path::Path::new(&corpus)).expect("build from corpus");
        producer.warm_up().expect("warm up");

        let mut all = Vec::new();
        let mut last = 0.0f32;
        let mut opened = 0;
        for frame in 0..5u32 {
            let mut s = producer
                .step_frame(SdControls::default().to_input(), 0)
                .expect("a frame");
            assert!(s.len() > 1, "frame {frame} drew nothing at all");
            assert_eq!(
                s[0].flags,
                flags::DISCONTINUITY,
                "frame {frame} must open with the GO strobe's recentre"
            );
            opened += 1;
            offset_onto(&mut s, f64::from(frame) * FRAME_SECONDS, &mut last);
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
        println!("{} samples over {:.4} s", all.len(), last);
    }

    #[test]
    fn offsetting_keeps_time_strictly_increasing_across_frames() {
        let mut last = 0.0f32;
        let mut all = Vec::new();
        for frame in 0..3 {
            let mut s = samples_of(&[BeamMove {
                x0: 0,
                y0: 0,
                x1: 64,
                y1: 0,
                intensity: 6,
                color: 7,
                jump: false,
            }]);
            offset_onto(&mut s, frame as f64 * FRAME_SECONDS, &mut last);
            all.extend(s);
        }
        for pair in all.windows(2) {
            assert!(pair[1].t > pair[0].t, "{} follows {}", pair[1].t, pair[0].t);
        }
        beam_trace::validate(&all).expect("a valid trace");
    }
}
