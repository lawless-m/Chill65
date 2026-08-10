//! **The gate.** Atari's own ROM diagnostic, run on our machine.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/crystal-castles \
//!   cargo test -p chill65-runtime --test selftest -- --ignored --nocapture
//! ```
//!
//! # Why this is the right gate
//!
//! It is binary, headless, and written by Atari rather than by us. "Playable"
//! cannot be asserted in a test; this can. Passing it means the CPU, the bus,
//! the ROM banking and the timing are all correct enough that the game's own
//! diagnostic agrees — and because the five 2764s span **both banks**, it
//! exercises bank-sensitive reads end to end.
//!
//! # How self-test is entered
//!
//! Not by a switch. `CIN.MAC:222-227` ends the power-up path on a
//! **conditional-assembly** branch, and `CG.MAC:9` sets `CG.ST = 1` in both the
//! root tree and `version-3`:
//!
//! ```text
//!     .IF NE,CG.ST
//!     .IFT
//!     JMP MN.ST       ; power-on self-test
//!     .IFF
//!     JMP MN.HIC      ; straight to the game
//!     .ENDC
//! ```
//!
//! So the shipped ROM **always** runs the self-test at power-up and no input is
//! needed. (The `HW.STS` switch at `CMN.MAC:772` is a separate runtime check
//! for re-entering self-test during play; an open switch reads bit 4 as 1 and
//! the game proceeds normally.)
//!
//! # What the ROM test computes
//!
//! `CST.MAC:312-323` walks both banks, 32 pages per 2764:
//!
//! ```text
//! ROMTST: TRAI 0 1+TEMP2      ;  checksum counter
//!         STA   HW.BSL        ;  bank zero ROM test
//!         LDX #20*NPROM1-1    ;  3 ROMs -> 96 pages, A000-FFFF
//!         JSR ROMTS1
//!         TRAI 0FF HW.BSL     ;  bank one ROM test
//!         LDX #20*NPROM2-1    ;  2 ROMs -> 64 pages, A000-DFFF
//!         JSR ROMTS1
//!         TRAI 0 HW.BSL
//!         JMP GOTCHK
//! ```
//!
//! `ROMTS1` accumulates a longitudinal parity (`EOR`) across each 8K ROM,
//! reseeding with `0FF` at every 32-page boundary and storing the result into
//! `TEMP3(Y)`. Note the `STA HW.WDC ; WOOF` once per page — that is what keeps
//! the watchdog quiet through a 40K scan.
//!
//! # The observable
//!
//! `CST.MAC:352-369`:
//!
//! ```text
//! GOTCHK: TRAI 0 1+TEMP2
//!         BEGIN
//!          LDY 1+TEMP2
//!          LDA TEMP3(Y)
//!          SUB #1
//!          CMP 1+TEMP2
//!          BNE 10$        ;  ROM bad
//!         INC 1+TEMP2
//!         ...
//!         CMP #5
//!         CSEND
//!         LDA #29         ;  ROM OK message
//!         JSR MS.DRW
//!         JMP MOREST
//! 10$:    LDA #17         ;  ROM bad -> draw, then
//!         ...
//! 17$:    BCS 17$         ;  spin forever and let the watchdog bite
//! ```
//!
//! So the pass condition is `TEMP3[i] - 1 == i`, i.e. the five checksums must be
//! exactly **1, 2, 3, 4, 5** — the index plus one, which also identifies *which*
//! ROM failed. Pass continues to `MOREST`; failure ends in an infinite spin.
//!
//! This test asserts all three: the checksum values, arrival at `MOREST`, and
//! that the failure path is never entered.

mod common;

use chill65_runtime::frame::{CYCLES_PER_LINE, FIRST_VISIBLE_LINE, LINES_PER_FRAME};
use chill65_runtime::{Bus, Cpu, CpuError, Machine};

// Addresses from the assembler's own symbol table (`--symbols`).
const ROMTST: u16 = 0xECC9;
const GOTCHK: u16 = 0xED17;
/// `10$` in `GOTCHK` — the ROM-bad path.
const ROM_BAD: u16 = 0xED37;
/// `17$` — `BCS 17$`, the terminal spin.
const FAIL_SPIN: u16 = 0xED7F;
/// Where a pass continues to.
const MOREST: u16 = 0xEDC6;
/// `TEMP3`, used as a five-byte checksum array (it overruns into `TEMP4`/`TEMP5`).
const TEMP3: u16 = 0x00AE;
const NUM_ROMS: usize = 5;

/// Generous: the self-test reaches `MOREST` well inside this on a healthy run.
const CYCLE_BUDGET: u64 = 20_000_000;

/// What the game's ROM test concluded.
#[derive(Debug)]
enum Verdict {
    /// Reached `MOREST`: all five checksums matched.
    Passed { checksums: [u8; NUM_ROMS] },
    /// Branched to `10$`: at least one ROM is wrong.
    Failed { checksums: [u8; NUM_ROMS] },
}

struct Run {
    verdict: Verdict,
    romtst_at: u64,
    gotchk_at: u64,
    banks_seen: (bool, bool),
    machine: Machine,
}

/// Boot the machine on these images and run until the ROM test reaches a
/// verdict.
fn run_rom_test(prog: &[u8], data: &[u8], mob: Option<&[u8]>) -> Run {
    let mut m = Machine::new();
    m.load_roms(prog, data).expect("load");
    if let Some(mob) = mob {
        m.load_motion_roms(mob).expect("motion roms");
    }
    let mut cpu = Cpu::new();
    cpu.reset(&mut m);

    let mut romtst_at = None;
    let mut gotchk_at = None;
    let mut checksums = [0u8; NUM_ROMS];
    let mut banks_seen = (false, false);

    while m.cycles < CYCLE_BUDGET {
        // The self-test runs entirely with interrupts masked — `MN.PWR` and
        // `MN.SLT` both `SEI`, and `CST.MAC` contains no `CLI` — so no
        // interrupt scheduling is needed here. VBLANK is another matter: the
        // "WAIT FOR 80 VBLANKS" loop at `CST.MAC:20` polls it, and the machine
        // would spin there forever if it never toggled.
        let line = (m.cycles / CYCLES_PER_LINE as u64) % LINES_PER_FRAME as u64;
        m.input.vblank = line < FIRST_VISIBLE_LINE as u64;

        match cpu.step(&mut m) {
            Ok(_) => {}
            Err(CpuError::IllegalOpcode { opcode, pc }) => {
                panic!(
                    "illegal opcode {opcode:#04x} at {pc:04X} after {} cycles",
                    m.cycles
                )
            }
        }

        // Record which bank was live during the scan, to prove both were used.
        if romtst_at.is_some() && gotchk_at.is_none() {
            if m.data_bank_selected() {
                banks_seen.1 = true;
            } else {
                banks_seen.0 = true;
            }
        }

        match cpu.pc {
            ROMTST if romtst_at.is_none() => romtst_at = Some(m.cycles),
            GOTCHK if gotchk_at.is_none() => {
                gotchk_at = Some(m.cycles);
                // Capture before GOTCHK's loop runs; TEMP3 is scratch that
                // later code reuses for other things entirely.
                for (i, c) in checksums.iter_mut().enumerate() {
                    *c = m.peek(TEMP3 + i as u16);
                }
            }
            ROM_BAD | FAIL_SPIN => {
                return Run {
                    verdict: Verdict::Failed { checksums },
                    romtst_at: romtst_at.unwrap_or(0),
                    gotchk_at: gotchk_at.unwrap_or(0),
                    banks_seen,
                    machine: m,
                }
            }
            MOREST => {
                return Run {
                    verdict: Verdict::Passed { checksums },
                    romtst_at: romtst_at.expect("ROMTST"),
                    gotchk_at: gotchk_at.expect("GOTCHK"),
                    banks_seen,
                    machine: m,
                }
            }
            _ => {}
        }
    }

    panic!(
        "no verdict within {CYCLE_BUDGET} cycles — PC {:04X}, ROMTST {romtst_at:?}, \
         GOTCHK {gotchk_at:?}",
        cpu.pc
    )
}

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn the_games_own_rom_test_passes() {
    let Some(corpus) = common::corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let (prog, data) = common::build_images(&corpus);
    let mob = common::motion_rom_image(&corpus);
    let run = run_rom_test(&prog, &data, Some(&mob));

    eprintln!(
        "self-test: ROMTST at {} cycles, GOTCHK at {} ({} cycles scanning 40K \
         across both banks)\n\x20 verdict {:?}, watchdog strobes {}, \
         bank writes {} ({} switches)",
        run.romtst_at,
        run.gotchk_at,
        run.gotchk_at - run.romtst_at,
        run.verdict,
        run.machine.watchdog_strobes,
        run.machine.bank_writes,
        run.machine.bank_switches,
    );

    let checksums = match run.verdict {
        Verdict::Passed { checksums } => checksums,
        Verdict::Failed { checksums } => panic!(
            "the game's ROM test FAILED: checksums {checksums:02X?}, expected \
             [01, 02, 03, 04, 05]. GOTCHK requires TEMP3[i] - 1 == i, so the \
             first mismatching index names the bad ROM: {:?}",
            checksums
                .iter()
                .enumerate()
                .find(|(i, &c)| c as usize != i + 1)
                .map(|(i, _)| i)
        ),
    };

    // The verdict GOTCHK itself applies, asserted directly so a failure names
    // the offending ROM rather than merely reporting a wrong branch.
    for (i, &c) in checksums.iter().enumerate() {
        assert_eq!(
            c as usize,
            i + 1,
            "ROM {i} checksum is {c:#04x}; GOTCHK requires TEMP3[{i}] - 1 == {i}"
        );
    }

    // Both banks must have been mapped during the scan, or the bank-sensitive
    // read path was never exercised and the result proves much less.
    assert!(run.banks_seen.0, "bank 0 was never live during the ROM scan");
    assert!(run.banks_seen.1, "bank 1 was never live during the ROM scan");
    assert!(
        run.machine.bank_switches >= 2,
        "the scan must switch to bank 1 and back"
    );
    assert!(
        !run.machine.watchdog_expired,
        "the watchdog bit during the ROM scan"
    );
}

/// The gate must be capable of failing, or passing it means nothing.
///
/// Corrupt one byte of the program image and the game's own diagnostic should
/// notice. This also pins *which* ROM it blames: byte 0 is in `A000-BFFF`,
/// which is the first 2764 of bank 0, so index 0 must be the mismatch.
#[test]
#[ignore = "needs CHILL65_CORPUS pointing at the game source"]
fn a_corrupted_rom_is_detected() {
    let Some(corpus) = common::corpus() else {
        return;
    };

    let (mut prog, data) = common::build_images(&corpus);
    let mob = common::motion_rom_image(&corpus);
    // Flip a bit well away from the vectors and the boot path, so the machine
    // still runs normally and the checksum is the only thing that changes.
    prog[0x0100] ^= 0x01;

    let run = run_rom_test(&prog, &data, Some(&mob));
    match run.verdict {
        Verdict::Failed { checksums } => {
            eprintln!("corrupted ROM correctly rejected: checksums {checksums:02X?}");
            assert_ne!(
                checksums[0], 1,
                "ROM 0 holds the corrupted byte, so its checksum must differ"
            );
        }
        Verdict::Passed { checksums } => panic!(
            "a corrupted ROM PASSED the self-test ({checksums:02X?}) — the gate \
             does not discriminate and proves nothing"
        ),
    }
}
