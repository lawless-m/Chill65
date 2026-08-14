//! Boot Tempest on the real ROMs and watch it keep drawing.
//!
//! The counterpart of `boot_sd.rs`. The image is byte-identical to
//! `TEMPST.LDA` (`gate_te`), so anything that goes wrong here is the board's
//! fault, not the bytes'.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/tempest \
//!   cargo test -p chill65-runtime --test boot_te -- --ignored --nocapture
//! ```
//!
//! # Why the budget is what it is
//!
//! `gate2.md` records the trap Space Duel set: its first draw is frame 98, and
//! the quiet stretch before it *is* the power-on self-test. A budget that
//! stopped inside it would see a blank screen and a healthy watchdog and be
//! unable to tell that from a wedge.
//!
//! Tempest has a quiet stretch of its own, and a documented one:
//! `ALTEST.MAC:283-289` spins for roughly 400 ms — about eleven game frames —
//! waiting for the EAROM supply to settle before `CLI` and `MAINLN`.
//!
//! Measured on this model, not guessed:
//!
//! - **frame 17** — the first interrupt is taken, the first `VGSTART` is
//!   strobed and the first segments appear, all together. The `CLI` at
//!   `ALTEST.MAC:292` is what releases all three at once, and it comes after
//!   the EAROM wait, so the quiet stretch is the boot rather than a wedge.
//! - **one `VGSTART` per interrupt** thereafter: 3,447 strobes against 3,447
//!   interrupts taken. That is `ALHARD.MAC:170-175` restarting the generator
//!   every time it finds the halt bit set, not one strobe a frame as Space
//!   Duel does.
//! - **365 segments** in a steady-state picture, and **343 distinct pictures**
//!   across 400 frames.
//!
//! 400 frames leaves the boot far behind and gives the attract sequence room
//! to animate many times over.
//!
//! # A vector machine wedges without crashing
//!
//! Also from `gate2.md`: if the display list stops being rebuilt, the
//! generator happily redraws the last one and the interrupt handler keeps
//! feeding the watchdog. A frame hash that stops changing is the only signal,
//! which is why this test counts distinct pictures rather than merely checking
//! that something was drawn.

mod common;

use chill65_runtime::bus::Bus;
use chill65_runtime::cpu::Cpu;
use chill65_runtime::te::{run_frame_te, TeMachine, CYCLES_PER_FRAME, IRQS_PER_FRAME};

use common::{build_te_program, corpus, TE_PROG_BASE, TE_PROG_LEN, TE_VEC_ROM_BASE,
             TE_VEC_ROM_LEN};

/// Frames to run.
///
/// Long enough to clear the EAROM settle stretch several times over and see
/// steady-state drawing with room to spare.
const SMOKE_FRAMES: u32 = 400;

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
fn the_real_roms_boot_and_run() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let program = build_te_program(&c);
    let vec_rom = program.bytes(TE_VEC_ROM_BASE, TE_VEC_ROM_LEN);
    let prog = program.bytes(TE_PROG_BASE, TE_PROG_LEN);

    let mut m = TeMachine::new();
    m.load_roms(&vec_rom, &prog).expect("load");
    // Self-test switch idle — the normal boot. `ALTEST.MAC:280-282` takes this
    // path when `IN1 & MTEST` is non-zero; `selftest_te.rs` holds it the other
    // way.
    m.input.self_test = false;

    // Before a single instruction: the vectors must land in program ROM. That
    // proves the top-page mirror and the link at once. Compared as ranges and
    // printed — no oracle bytes become constants here.
    let reset = m.read_u16(0xFFFC);
    let irq = m.read_u16(0xFFFE);
    println!("reset vector {reset:04X}, IRQ vector {irq:04X}");
    assert!(
        (0x9000..=0xDFFF).contains(&reset),
        "reset vector {reset:04X} is outside program ROM"
    );
    assert!(
        (0x9000..=0xDFFF).contains(&irq),
        "IRQ vector {irq:04X} is outside program ROM"
    );

    let mut cpu = Cpu::new();
    cpu.reset(&mut m);
    assert_eq!(cpu.pc, reset, "the CPU starts at the reset vector");

    let mut irqs_taken = 0u64;
    let mut first_irq = None;
    let mut first_go = None;
    let mut first_draw = None;
    let mut hashes = Vec::new();

    for frame in 0..SMOKE_FRAMES {
        let stats = run_frame_te(&mut cpu, &mut m)
            .unwrap_or_else(|e| panic!("CPU error on frame {frame}: {e:?}"));
        irqs_taken += stats.irqs_taken as u64;

        if first_irq.is_none() && stats.irqs_taken > 0 {
            first_irq = Some(frame);
        }
        if first_go.is_none() && m.vg_go_strobes > 0 {
            first_go = Some(frame);
        }
        if first_draw.is_none() && !m.segments.is_empty() {
            first_draw = Some(frame);
        }
        hashes.push(m.frame_hash());

        assert_eq!(stats.irqs_raised, IRQS_PER_FRAME, "frame {frame}");
        assert!(
            !m.watchdog_expired,
            "the watchdog expired on frame {frame}; the machine wedged"
        );
        assert!(
            m.vg_fault.is_none(),
            "vector generator fault on frame {frame}: {:?}",
            m.vg_fault
        );
    }

    let distinct = {
        let mut h = hashes.clone();
        h.sort_unstable();
        h.dedup();
        h.len()
    };
    println!(
        "{SMOKE_FRAMES} frames, {} cycles\n  \
         first IRQ taken   frame {first_irq:?}\n  \
         first VGSTART     frame {first_go:?}\n  \
         first segments    frame {first_draw:?}\n  \
         IRQs taken        {irqs_taken}\n  \
         GO strobes        {}\n  \
         watchdog strobes  {}\n  \
         segments now      {}\n  \
         distinct hashes   {distinct}",
        m.cycles,
        m.vg_go_strobes,
        m.watchdog_strobes,
        m.segments.len(),
    );

    assert!(
        m.cycles >= SMOKE_FRAMES as u64 * CYCLES_PER_FRAME as u64,
        "ran {} cycles, expected at least one frame's worth each",
        m.cycles
    );
    assert!(irqs_taken > 0, "no interrupt was ever taken");
    assert!(
        m.watchdog_strobes > 0,
        "the game never fed the watchdog; ALHARD.MAC:56 strobes it every IRQ"
    );
    assert!(
        m.vg_go_strobes > 0,
        "the game never started the vector generator"
    );
    assert!(
        !m.segments.is_empty(),
        "nothing was drawn by frame {SMOKE_FRAMES}"
    );
    assert!(
        distinct > 1,
        "the picture never changed across {SMOKE_FRAMES} frames — \
         a vector machine wedges without crashing, and this is the only signal"
    );
}
