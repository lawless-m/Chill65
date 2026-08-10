//! Boot smoke test: assemble the real ROMs and run the machine on them.
//!
//! # Why this is gated
//!
//! The game source is not in this repository and never will be — plan §9: we
//! ship the toolchain, the user supplies their own source. So this test is
//! `#[ignore]`d and gated on `CHILL65_CORPUS`, and the default `cargo test`
//! stays green with no corpus present.
//!
//! The images it builds are **build artefacts containing game-derived bytes**.
//! They are written under `target/`, which is gitignored, and must never be
//! committed.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-runtime --test boot -- --ignored --nocapture
//! ```

mod common;

use chill65_runtime::{frame::run_frame, Bus, Cpu, Machine};

use common::{build_images, corpus, motion_rom_image, DATA_LEN, PROG_LEN};

/// How many frames to run.
///
/// This is not arbitrary, and 500,000 cycles (25 frames) is not enough to prove
/// anything. `CG.MAC:9` sets `CG.ST = 1`, so the ROM's power-up path is
/// `MN.PWR -> MN.SLT -> MN.ST` — it **always** runs the power-on self-test
/// first, which includes `CST.MAC:20`'s "WAIT FOR 80 VBLANKS" (127 of them) and
/// a checksum over both ROM banks. Interrupts stay masked throughout.
///
/// Measured on this model: the first frame that draws anything is 162, and the
/// first interrupt is taken at frame 353, when the game finally reaches
/// `MN.HIC`'s `CLI` (`CMN.MAC:32`). Running only to 500,000 cycles stops in the
/// middle of the self-test, where "no interrupts and a blank screen" is correct
/// but indistinguishable from a wedged machine.
const SMOKE_FRAMES: u32 = 420;

/// The task's floor, which [`SMOKE_FRAMES`] comfortably exceeds.
const MIN_CYCLES: u64 = 500_000;

#[test]
fn load_roms_rejects_wrong_sizes() {
    // Not corpus-dependent, so this one runs by default.
    let mut m = Machine::new();
    assert!(m.load_roms(&[0; PROG_LEN], &[0; DATA_LEN]).is_ok());

    let e = m.load_roms(&[0; 1000], &[0; DATA_LEN]).unwrap_err();
    assert!(e.contains("24576"), "{e}");
    let e = m.load_roms(&[0; PROG_LEN], &[0; 1000]).unwrap_err();
    assert!(e.contains("16384"), "{e}");
}

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn the_real_roms_boot_and_run() {
    let Some(corpus) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let (prog, data) = build_images(&corpus);
    let mut m = Machine::new();
    m.load_roms(&prog, &data).expect("load");
    m.load_motion_roms(&motion_rom_image(&corpus))
        .expect("motion roms");

    // --- Vectors -----------------------------------------------------------
    let vector = |m: &mut Machine, a: u16| m.read_u16(a);
    assert_eq!(vector(&mut m, 0xFFFC), 0xE000, "RESET");
    assert_eq!(vector(&mut m, 0xFFFE), 0xE009, "IRQ");
    assert_eq!(
        vector(&mut m, 0xFFFA),
        0xE009,
        "NMI vector is populated even though MicroProcessor.v:26 ties the line low"
    );

    // --- The bank-sniff invariant ------------------------------------------
    // CIN.MAC:35-41 tells the banks apart by reading A000 and testing for
    // zero. That only works because of a content invariant spanning two
    // separate assemblies, so assert it directly rather than trusting it.
    assert_eq!(data[0], 0x00, "castle data must begin with zero");
    assert_ne!(prog[0], 0x00, "program must not begin with zero");
    let banner = String::from_utf8_lossy(&prog[..32]);
    assert!(
        banner.contains("ATARI"),
        "expected the copyright string at A000, found {banner:?}"
    );

    // --- Boot and run ------------------------------------------------------
    let mut cpu = Cpu::new();
    cpu.reset(&mut m);
    assert_eq!(cpu.pc, 0xE000, "reset vector followed");

    let blank = m.frame_hash();
    let mut irqs = 0u32;
    let mut first_draw = None;
    let mut first_irq = None;
    let mut distinct_images = 0u32;
    let mut last_hash = blank;

    for frame in 0..SMOKE_FRAMES {
        let stats = run_frame(&mut cpu, &mut m).unwrap_or_else(|e| {
            panic!(
                "{e} at frame {frame} ({} cycles), PC {:04X}",
                m.cycles, cpu.pc
            )
        });
        irqs += stats.irqs_taken;
        if stats.irqs_taken > 0 && first_irq.is_none() {
            first_irq = Some(frame);
        }
        let hash = m.frame_hash();
        if hash != last_hash {
            distinct_images += 1;
            last_hash = hash;
            if first_draw.is_none() {
                first_draw = Some(frame);
            }
        }
    }

    // Background is colour RAM entry 16 now, not index 0.
    let lit = m
        .framebuffer()
        .iter()
        .filter(|&&p| p != chill65_runtime::video::BITMAP_CRAM_BASE as u8)
        .count();
    eprintln!(
        "booted: {SMOKE_FRAMES} frames, {} cycles, {irqs} interrupts taken\n\
         \x20 first draw: frame {first_draw:?}, first interrupt: frame {first_irq:?}\n\
         \x20 {distinct_images} distinct images, {lit} lit pixels, \
         {} watchdog strobes, {} bank writes ({} switches)",
        m.cycles,
        m.watchdog_strobes,
        m.bank_writes,
        m.bank_switches,
    );

    // --- The task's floor ---------------------------------------------------
    assert!(m.cycles >= MIN_CYCLES, "ran only {} cycles", m.cycles);
    assert!(
        m.watchdog_strobes > 0,
        "nothing fed the watchdog — the machine is not executing the game"
    );
    assert!(
        m.bank_writes > 0,
        "no write to HW.BSL (9E87) observed — the bank latch is never exercised"
    );
    assert!(!m.watchdog_expired, "the watchdog went hungry");

    // --- Evidence it actually booted, rather than merely not crashing -------
    assert!(
        first_draw.is_some(),
        "the framebuffer never changed — the machine ran without drawing anything"
    );
    assert!(
        lit > 0,
        "the visible picture is entirely blank at the end of the run"
    );
    assert!(
        first_irq.is_some(),
        "no interrupt was ever taken — the game never reached MN.HIC's CLI, so it \
         did not get past the power-on self-test"
    );
    assert!(
        m.bank_switches > 0,
        "the bank never actually changed — CDB.MAC's castle parser never ran"
    );
}
