//! Independent CPU validation against Klaus Dormann's 6502 functional tests.
//!
//! # Licence position
//!
//! The test suite is **GPL v3** (`license.txt` in its repository). It is not
//! copied into this repository, not redistributed, and not linked against — it
//! is a data file this harness *runs*, exactly as one would run any external
//! program. Running a GPL work places no obligation on the runner. The clone
//! lives outside the tree and is reached through an environment variable.
//!
//! # Running it
//!
//! ```text
//! git clone https://github.com/Klaus2m5/6502_65C02_functional_tests
//! CHILL65_KLAUS=/path/to/6502_65C02_functional_tests \
//!   cargo test -p chill65-runtime --test klaus -- --ignored --nocapture
//! ```
//!
//! # How the suite signals
//!
//! Both success and failure are `jmp *` — a branch to itself. So the harness
//! runs until the PC repeats, then asks *which* address it settled on. The
//! success trap is at `$3469`, read from `bin_files/6502_functional_test.lst`:
//!
//! ```text
//! 3469 : 4c6934          >        jmp *           ;test passed, no errors
//! ```
//!
//! Any other resting place is a failure, and the listing can be searched for
//! that address to find which test gave up.

use std::path::PathBuf;

use chill65_runtime::{Bus, Cpu, CpuError, FlatBus};

/// Where the suite parks when every test has passed.
const SUCCESS_TRAP: u16 = 0x3469;
/// The suite is linked to run from here.
const ENTRY: u16 = 0x0400;
/// Generous: a full pass is tens of millions of cycles, not billions.
const MAX_INSTRUCTIONS: u64 = 500_000_000;

fn corpus() -> Option<PathBuf> {
    std::env::var("CHILL65_KLAUS").ok().map(PathBuf::from)
}

/// Run a test image until the PC stops moving. Returns the resting address.
fn run_to_trap(image: &[u8], entry: u16) -> Result<(u16, u64), String> {
    assert_eq!(image.len(), 0x10000, "the suite images are full 64K");

    let mut bus = FlatBus::new();
    bus.mem.copy_from_slice(image);

    let mut cpu = Cpu::new();
    cpu.pc = entry;
    // The suite manages its own stack and flags; it does not want a reset
    // vector fetch, which would land it somewhere arbitrary.

    let mut executed = 0u64;
    loop {
        let before = cpu.pc;
        match cpu.step(&mut bus) {
            Ok(_) => {}
            Err(CpuError::IllegalOpcode { opcode, pc }) => {
                return Err(format!(
                    "illegal opcode {opcode:#04x} at {pc:04X} after {executed} instructions"
                ));
            }
        }
        executed += 1;

        // `jmp *` — the PC has not moved, so the machine has parked.
        if cpu.pc == before {
            return Ok((cpu.pc, executed));
        }

        if executed >= MAX_INSTRUCTIONS {
            return Err(format!(
                "no trap reached after {executed} instructions; last PC {:04X}",
                cpu.pc
            ));
        }
    }
}

#[test]
#[ignore = "needs CHILL65_KLAUS pointing at a local clone of the suite"]
fn functional_test_suite_passes() {
    let Some(root) = corpus() else {
        eprintln!("CHILL65_KLAUS unset — skipping");
        return;
    };

    let path = root.join("bin_files/6502_functional_test.bin");
    let image = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));

    match run_to_trap(&image, ENTRY) {
        Ok((trap, executed)) => {
            eprintln!("Klaus functional test: parked at {trap:04X} after {executed} instructions");
            assert_eq!(
                trap, SUCCESS_TRAP,
                "parked at {trap:04X}, not the success trap {SUCCESS_TRAP:04X} — \
                 search bin_files/6502_functional_test.lst for that address to \
                 find which test gave up"
            );
        }
        Err(e) => panic!("Klaus functional test: {e}"),
    }
}

/// The decimal-mode test.
///
/// It matters more than most: NMOS decimal flags are the subtlest thing the CPU
/// does — `ADC` takes Z from the *binary* sum while `SBC` takes every flag from
/// the binary operation — and `CCN.MAC`'s coin counting and `CAL.MAC`'s scoring
/// both run with D set. This test walks all 256×256 operand pairs for both
/// carry-in values, in both add and subtract, checking accumulator *and* every
/// flag against independently computed predictions.
///
/// The suite ships it as `.a65` source only. Its own bundled assembler
/// (`as65_142.zip`) is a 32-bit Linux binary and does the job:
///
/// ```text
/// unzip as65_142.zip as65 && chmod +x as65
/// ./as65 -l -m -w -h0 6502_decimal_test.a65
/// mv 6502_decimal_test.bin bin_files/
/// ```
///
/// It signals differently from the functional test: code sits at `$0200`, and
/// it ends with `db $db` — a 65C02 `STP`, which on NMOS is an illegal opcode.
/// So *reaching* an illegal `$DB` is the completion signal, and the verdict is
/// then the `ERROR` byte at `$000b`: zero for pass, one for fail.
#[test]
#[ignore = "needs CHILL65_KLAUS and an assembled bin_files/6502_decimal_test.bin"]
fn decimal_test_suite_passes() {
    let Some(root) = corpus() else {
        return;
    };
    let path = root.join("bin_files/6502_decimal_test.bin");
    let Ok(code) = std::fs::read(&path) else {
        eprintln!(
            "no {} — assemble it with the suite's own as65; skipping",
            path.display()
        );
        return;
    };

    const CODE_ORG: u16 = 0x0200;
    const ERROR_FLAG: u16 = 0x000B;
    const STP: u8 = 0xDB;

    let mut bus = FlatBus::new();
    bus.load(CODE_ORG, &code);
    let mut cpu = Cpu::new();
    cpu.pc = CODE_ORG;

    let mut executed = 0u64;
    let finished = loop {
        let before = cpu.pc;
        match cpu.step(&mut bus) {
            Ok(_) => {}
            // `db $db` — the test has run to completion.
            Err(CpuError::IllegalOpcode { opcode: STP, .. }) => break true,
            Err(e) => panic!("decimal test: {e} after {executed} instructions"),
        }
        executed += 1;
        if cpu.pc == before {
            break true; // parked, however it got there
        }
        assert!(
            executed < MAX_INSTRUCTIONS,
            "decimal test did not finish; last PC {:04X}",
            cpu.pc
        );
    };
    assert!(finished);

    let error = bus.read(ERROR_FLAG);
    eprintln!("Klaus decimal test: finished after {executed} instructions, ERROR={error}");
    assert_eq!(
        error, 0,
        "decimal-mode failure — ERROR is 1. The suite leaves the failing case in \
         zero page: N1={:#04x} N2={:#04x}, accumulator got {:#04x} expected {:#04x}",
        bus.read(0x00),
        bus.read(0x01),
        bus.read(0x04),
        bus.read(0x06),
    );
}
