//! The machine running in real time, feeding a ring buffer.
//!
//! The window draws whenever the display lets it; the machine runs at its own
//! rate — 61.5234 Hz for Space Duel, 27.78 Hz for Tempest — whatever the window
//! is doing. So the two are separated by a ring buffer and a thread, and share
//! exactly one thing: the clock. `elapsed()` is the renderer's `T_now` *and*
//! the origin the sample timestamps are placed against, which is what keeps the
//! beam on the tube in step with the game.
//!
//! This mirrors `tube-shell`'s own producer (`source.rs`), including the small
//! things that matter: run a little ahead of the renderer but never far ahead,
//! sleep rather than spin when there is nothing to do, and nudge a timestamp
//! that would repeat rather than emitting one the format forbids.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use beam_trace::{RingBuffer, Sample, DEFAULT_CAPACITY};

use crate::machine::{offset_onto, Controls, Game, Producer};

/// How far ahead of the renderer the machine may run.
const LOOKAHEAD_SECONDS: f64 = 0.05;

/// How long to sleep when there is nothing to produce.
const IDLE: Duration = Duration::from_millis(2);

/// A machine running on its own thread, and the beam it has produced.
pub struct LiveMachine {
    ring: Arc<Mutex<RingBuffer>>,
    controls: Arc<Mutex<Controls>>,
    status: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    start: Instant,
    game: Game,
}

impl LiveMachine {
    /// Start the machine. Assembling the ROMs takes a few seconds and happens
    /// on the thread, so this returns at once and [`Self::status`] reports
    /// progress.
    pub fn spawn(game: Game, corpus: PathBuf) -> Self {
        let ring = Arc::new(Mutex::new(RingBuffer::with_capacity(DEFAULT_CAPACITY, 0.0)));
        let controls = Arc::new(Mutex::new(Controls::new(game)));
        let status = Arc::new(Mutex::new("assembling the game…".to_owned()));
        let stop = Arc::new(AtomicBool::new(false));
        let start = Instant::now();

        let thread = std::thread::Builder::new()
            .name(game.title().to_owned())
            .spawn({
                let ring = Arc::clone(&ring);
                let controls = Arc::clone(&controls);
                let status = Arc::clone(&status);
                let stop = Arc::clone(&stop);
                move || produce(game, &corpus, &ring, &controls, &status, &stop, start)
            })
            .expect("spawn the machine thread");

        LiveMachine {
            ring,
            controls,
            status,
            stop,
            thread: Some(thread),
            start,
            game,
        }
    }

    pub fn game(&self) -> Game {
        self.game
    }

    /// Seconds since the machine started. The renderer's `T_now`.
    pub fn elapsed(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// Everything needed to draw the interval `[from, to]`.
    pub fn window(&self, from: f32, to: f32) -> Vec<Sample> {
        let ring = self.ring.lock().expect("ring lock");
        let (a, b) = ring.spans_in(from, to);
        let mut out = Vec::with_capacity(a.len() + b.len());
        out.extend_from_slice(a);
        out.extend_from_slice(b);
        out
    }

    /// Samples currently held.
    pub fn buffered(&self) -> usize {
        self.ring.lock().expect("ring lock").len()
    }

    /// What the machine is doing, or why it is not.
    pub fn status(&self) -> String {
        self.status.lock().expect("status lock").clone()
    }

    /// Change the switches. The window calls this on every key event.
    pub fn set_controls(&self, c: Controls) {
        *self.controls.lock().expect("controls lock") = c;
    }

    pub fn controls(&self) -> Controls {
        *self.controls.lock().expect("controls lock")
    }
}

impl Drop for LiveMachine {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn produce(
    game: Game,
    corpus: &std::path::Path,
    ring: &Mutex<RingBuffer>,
    controls: &Mutex<Controls>,
    status: &Mutex<String>,
    stop: &AtomicBool,
    start: Instant,
) {
    let report = |text: String| *status.lock().expect("status lock") = text;

    let mut producer = match Producer::from_corpus(game, corpus) {
        Ok(p) => p,
        Err(e) => {
            report(e.clone());
            eprintln!("chill65-window: {e}");
            return;
        }
    };
    report("warming up…".to_owned());
    if let Err(e) = producer.warm_up() {
        report(e.clone());
        return;
    }
    report("running".to_owned());

    let frame_seconds = game.frame_seconds();
    let coin_hold_frames = game.coin_hold_frames();
    let mut cursor = start.elapsed().as_secs_f64();
    let mut last_t = 0.0f32;
    let mut coins_consumed = 0u64;
    let mut coin_hold = 0u32;

    while !stop.load(Ordering::Relaxed) {
        let now = start.elapsed().as_secs_f64();
        if cursor > now + LOOKAHEAD_SECONDS {
            std::thread::sleep(IDLE);
            continue;
        }

        let controls = *controls.lock().expect("controls lock");
        // A key tap is far shorter than the coin mech's debounce, so each
        // requested insertion latches the line for long enough to register.
        if coin_hold == 0 && coins_consumed < controls.coin_pulses() {
            coins_consumed += 1;
            coin_hold = coin_hold_frames;
        }
        let coin = coin_hold > 0;
        coin_hold = coin_hold.saturating_sub(1);

        let mut samples = match producer.step_frame(&controls, coin) {
            Ok(s) => s,
            Err(e) => {
                report(format!("the machine stopped: {e}"));
                eprintln!("chill65-window: {e}");
                return;
            }
        };
        offset_onto(&mut samples, cursor, &mut last_t);
        cursor += frame_seconds;

        let mut ring = ring.lock().expect("ring lock");
        for s in samples {
            ring.push(s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The machine keeps up with the wall clock, and neither starves the
    /// renderer nor races away from it.
    #[test]
    fn it_runs_in_real_time_and_takes_a_coin() {
        let Ok(corpus) = std::env::var("CHILL65_CORPUS") else {
            eprintln!("CHILL65_CORPUS unset — skipping the live test");
            return;
        };
        let corpus = PathBuf::from(corpus);
        let game = Game::detect(&corpus).expect("a game");
        let live = LiveMachine::spawn(game, corpus);

        // Assembling and warming up takes a few seconds.
        let waited = Instant::now();
        while live.buffered() == 0 {
            assert!(
                waited.elapsed() < Duration::from_secs(60),
                "no samples after 60 s: {}",
                live.status()
            );
            std::thread::sleep(Duration::from_millis(100));
        }

        let first = live.buffered();
        std::thread::sleep(Duration::from_millis(700));
        assert!(
            live.buffered() > first,
            "the ring stopped filling: {}",
            live.status()
        );

        let now = live.elapsed();
        let window = live.window(0.0, now as f32);
        assert!(!window.is_empty());
        beam_trace::validate(&window).expect("a valid trace");

        let newest = window.last().expect("samples").t as f64;
        assert!(
            newest <= now + LOOKAHEAD_SECONDS + 0.5,
            "the machine ran away: newest sample at {newest:.3} s, clock at {now:.3} s"
        );
        assert!(
            newest > now - 1.0,
            "the machine fell behind: newest sample at {newest:.3} s, clock at {now:.3} s"
        );

        // Insert a coin and press start; the beam must keep flowing.
        let mut c = live.controls();
        c.insert_coin();
        match &mut c {
            Controls::Sd(sd) => sd.start = true,
            Controls::Te(te) => te.start1 = true,
        }
        live.set_controls(c);
        let before = live.buffered();
        std::thread::sleep(Duration::from_millis(700));
        assert!(live.buffered() >= before, "the machine stopped on a coin");
        let window = live.window(0.0, live.elapsed() as f32);
        beam_trace::validate(&window).expect("still a valid trace after a coin");
        println!(
            "{}: {} samples buffered, status: {}",
            game.title(),
            live.buffered(),
            live.status()
        );
    }
}
