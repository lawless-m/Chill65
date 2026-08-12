//! Run Space Duel's own self-test and read out its verdict.
//!
//! # What the self-test checks
//!
//! `AS2TST.MAC` is not only the interactive test screen: it also holds
//! `POWERON` (`:66`), which **is** the reset vector, so its checks run on
//! every boot before the game starts. In order (`.SBTTL` headings):
//!
//! ```text
//! :170  ZERO-PAGE TEST
//! :253  VG RAM TEST
//! :354  ROM TEST
//! :423  TEST POKEY & EAROM
//! :467  REPORT PROBLEMS & DISPLAY SWITCHES
//! ```
//!
//! # How the verdict is observable
//!
//! `AS2TST.MAC:414-420` documents its own result block:
//!
//! ```text
//! (PNTTBL,+6) = 7 CHKSUMS FOR ROMS
//! (ERPLC,+3)  = ERROR FLAGS FOR:
//!     ERPLC:      RAM ERROR
//!     ERPLC+1:    POKEY1
//!     ERPLC+2:    POKEY2
//!     ERPLC+3:    EAROM
//! ```
//!
//! The checksums are EOR sums seeded by the `CKUM*` bytes planted through the
//! source — `ASTRD2.MAC` puts `CKUM2` at the head of the `4000` page,
//! `AS2POK.MAC` `CKUM5` at `70C0`, `AS2TST.MAC` `CKUM6` at `802C` — so a
//! correct ROM sums to **zero**. `AS2TST.MAC:404-406` relies on exactly that:
//! `LDA PNTTBL / ORA PNTTBL+1 / IFNE` sounds an alarm tone when either is
//! non-zero.
//!
//! So this test is stronger than a smoke test. It runs the game's own
//! arithmetic over the whole image and asserts the game is satisfied with it —
//! an independent check on the Phase 1 build, using the original's own seeds.
//!
//! # Why this is gated
//!
//! The game source is not in this repository — plan §9.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/space-duel \
//!   cargo test -p chill65-runtime --test selftest_sd -- --ignored --nocapture
//! ```

mod common;

use chill65_runtime::sd::{run_frame_sd, SdMachine};
use chill65_runtime::{Bus, Cpu};

use common::{build_sd_program, build_sd_ship, corpus, SD_PROG_LEN};

/// How many frames to run.
///
/// Measured: the checksums settle by frame 98, the same point at which the
/// boot test sees the first `GOADD` — the quiet stretch before anything is
/// drawn *is* the power-on test. 200 frames leaves margin and lets the test
/// screen come up and animate.
const TEST_FRAMES: u32 = 200;

/// Both option bytes zero, as in `boot_sd.rs`.
const OPTN1: u8 = 0x00;
const OPTN2: u8 = 0x00;

#[test]
#[ignore = "needs CHILL65_CORPUS"]
fn the_self_test_passes_its_own_checks() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let ship = build_sd_ship(&c);
    let program = build_sd_program(&c);
    let image = program.bytes(0x0000, SD_PROG_LEN);

    // Resolved from the build, not hard-coded: `PNTTBL` and `ERPLC` are
    // locals of AS2TST, and if the link ever moves them the test follows.
    let pnttbl = program.symbol("PNTTBL").expect("PNTTBL");
    let erplc = program.symbol("ERPLC").expect("ERPLC");
    println!("PNTTBL at {pnttbl:04X}, ERPLC at {erplc:04X}");

    let mut m = SdMachine::new();
    m.load_roms(&ship, &image).expect("load");
    m.set_options(OPTN1, OPTN2);
    // Hold the self-test switch on. `AS2DEC.MAC:9` says 0 = ON, and
    // `ASTRD2.MAC:719-722` jumps to `STESTA` when the masked bit is clear.
    m.self_test = true;

    let mut cpu = Cpu::new();
    cpu.reset(&mut m);

    for frame in 0..TEST_FRAMES {
        run_frame_sd(&mut cpu, &mut m)
            .unwrap_or_else(|e| panic!("CPU error on frame {frame}: {e:?}"));
        assert!(
            !m.watchdog_expired,
            "the watchdog expired on frame {frame} — the self-test wedged"
        );
        assert!(
            m.vg_fault.is_none(),
            "vector generator fault on frame {frame}: {:?}",
            m.vg_fault
        );
    }

    let checksums: Vec<u8> = (0..7).map(|i| m.peek(pnttbl + i)).collect();
    let flags: Vec<u8> = (0..4).map(|i| m.peek(erplc + i)).collect();
    println!(
        "after {TEST_FRAMES} frames\n  \
         ROM checksums   {checksums:02X?}\n  \
         RAM / POKEY1 / POKEY2 / EAROM   {flags:02X?}\n  \
         GO strobes      {}\n  \
         segments        {}",
        m.vg_go_strobes,
        m.segments.len()
    );

    // The machine got as far as drawing the test screen.
    assert!(m.vg_go_strobes > 0, "the self-test never drew anything");
    assert!(!m.segments.is_empty(), "the test screen is empty");

    // The verdict. A non-zero checksum means the game disagrees with the
    // image the Phase 1 assembler produced, which would be a serious finding
    // about the build rather than about this test.
    assert_eq!(
        checksums,
        vec![0u8; 7],
        "ROM checksums must all be zero; the game sounds an alarm otherwise \
         (AS2TST.MAC:404-406)"
    );
    assert_eq!(
        flags[0], 0,
        "the RAM test failed (AS2TST.MAC:351 stores a flag in ERPLC)"
    );
    assert_eq!(flags[1], 0, "POKEY 1 failed (AS2TST.MAC:431)");
    assert_eq!(flags[2], 0, "POKEY 2 failed");
    assert_eq!(flags[3], 0, "the EAROM test failed");
}
