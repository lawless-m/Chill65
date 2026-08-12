//! Run Space Duel's real display lists through the vector generator.
//!
//! # Why this test can exist at all
//!
//! The Phase 1 assembler reproduces Space Duel's images byte-identically
//! (`gate1.md`), so every display list in the built image is **known-correct
//! input**. That is an unusually strong position: the vector generator can be
//! checked against real data before any of the machine around it exists, and a
//! disagreement can only be the decoder's fault, never the data's.
//!
//! `vg.rs`'s own tests encode their words from the `VGMC.MAC` formulas, so they
//! check the decoder against the specification. This test checks it against
//! what Atari actually shipped. The two can fail independently, which is the
//! point of having both.
//!
//! # Why this is gated
//!
//! The game source is not in this repository and never will be — plan §9. So
//! this test is `#[ignore]`d and gated on `CHILL65_CORPUS`, and the default
//! `cargo test` stays green with no corpus present. The images it builds are
//! game-derived and live only in memory.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/space-duel \
//!   cargo test -p chill65-runtime --test vg_lists -- --ignored --nocapture
//! ```

mod common;

use chill65_runtime::vg::{Segment, Stop, Vg, STACK_DEPTH, VEC_BASE, VEC_BYTES};

use common::{
    build_sd_program, build_sd_ship_full, corpus, SdBuild, SD_SHIP_BASE, SD_SHIP_LEN,
};

/// How the board populates the generator's 8K window.
///
/// `AS2DEC.MAC:64-66` gives `VECRAM=2000` and `VECROM=2800`, and MAME's own
/// `spacduel` ROM layout agrees exactly — `136006-106.r7` loads at `2800`
/// (2048 bytes) and `136006-107.np7` at `3000` (4096 bytes). So:
///
/// ```text
/// 2000-27FF   vector RAM    the CPU's display list; empty here
/// 2800-2FFF   ship ROM      from the A2SHIP link
/// 3000-3FFF   vector ROM    from the fifteen-module link
/// ```
fn vector_window(ship: &[u8], program: &SdBuild) -> Vec<u8> {
    let mut mem = vec![0u8; VEC_BYTES];
    let ship_off = (SD_SHIP_BASE - VEC_BASE) as usize;
    mem[ship_off..ship_off + SD_SHIP_LEN].copy_from_slice(ship);
    let rom = program.bytes(0x3000, 0x1000);
    mem[0x1000..0x2000].copy_from_slice(&rom);
    mem
}

/// The budget for one list. Any real shape terminates in far fewer; this is
/// only here so a decode fault reports rather than hangs.
const BUDGET: u64 = 65_536;

struct Run {
    stop: Stop,
    steps: u64,
    max_depth: usize,
    segments: Vec<Segment>,
}

/// Execute one list from `addr`, stepping so nesting depth can be watched.
///
/// The generator starts with intensity latched to 7. These shapes are drawn by
/// the game through `VGUTR2.MAC`'s `VGSTAT`, which sets brightness before the
/// shape is called, and most of their vectors use `ZZ=1` — "use the latched
/// value" (`VGUTR2.MAC:71-72`). Starting from a dark latch would render them
/// invisible for reasons that have nothing to do with the decoder.
fn run_list(mem: &[u8], addr: u16) -> Run {
    assert!(
        (VEC_BASE..VEC_BASE + VEC_BYTES as u16).contains(&addr),
        "{addr:04X} is outside the vector window"
    );
    let mut vg = Vg::new();
    vg.reset((addr - VEC_BASE) / 2);
    vg.intensity = 7;
    let mut segments = Vec::new();
    let mut max_depth = 0;
    let mut stop = Stop::Budget;
    for _ in 0..BUDGET {
        max_depth = max_depth.max(vg.depth());
        if let Some(s) = vg.step(mem, &mut segments) {
            stop = s;
            break;
        }
    }
    Run {
        stop,
        steps: vg.executed,
        max_depth,
        segments,
    }
}

/// Display lists in the vector ROM, resolved from the program link's symbols.
///
/// Every name here was confirmed to resolve inside `3000-3FFF`. They are
/// looked up rather than hard-coded to addresses: a symbol table is not
/// game-derived data in the sense that matters, and if the link ever moves
/// them the test follows.
const ROM_LISTS: [&str; 15] = [
    "BOXES",  // attract-mode outer box
    "BOXCB",  // cabaret box
    "FRCFLD", // force field
    "WNDSET", // window setup
    "RHTSHP", // right-hand ship
    "LFTSHP", // left-hand ship
    "PAIRUD", // paired up/down ships
    "SPCONT", // space container
    "EXPSHP", // ship explosion
    "COMET", "DWARF", "SAUCER", "MINE", "SHOT", "SHIELD",
];

/// Character glyphs, which are display lists too.
///
/// `VGAN.MAC` defines one per character as a run of `VCTR`s ending in `RTSL`,
/// and `AS2ROM.MAC:246` includes it, so they land in the same vector ROM.
/// `ALPHA` (`VGMC.MAC:138-142`) draws a string by emitting one `JSRL` per
/// character at these labels.
///
/// `CHAR.` is the blank, and `VGAN.MAC:129` defines it as `VCTR 24,0,0` —
/// intensity zero. It must execute and move the beam without drawing, which
/// is why it is here but not in [`MUST_DRAW`].
const GLYPH_LISTS: [&str; 4] = ["CHAR.A", "CHAR.C", "CHAR.E", "CHAR."];

/// Lists that are shapes and must therefore draw something. `CNTSCL` is
/// excluded deliberately — it only centres the beam and sets scale — and so
/// is `CHAR.`, the blank glyph.
const MUST_DRAW: [&str; 14] = [
    "BOXES", "BOXCB", "RHTSHP", "LFTSHP", "PAIRUD", "SPCONT", "COMET", "DWARF", "SAUCER",
    "MINE", "SHOT", "CHAR.A", "CHAR.C", "CHAR.E",
];

// The ship ROM at 2800-2FFF is deliberately **not** executed here.
//
// It looks like it should be: `A2SHIP.MAC:47` exports `SHIPS`, and the board
// maps the image straight into the generator's window. But `SHIPS` is a table
// of `.WORD` pointers, and the shapes it points at are built with `TWBYPIC`,
// which `A2SHIP.MAC:36-38` defines as `.BYTE YY,XX` — two bytes per point, in
// a format of the game's own. The CPU reads them and builds vectors through
// `VGUTR2.MAC`'s `VGVCTR`; the generator never fetches them as instructions.
//
// Feeding them to the generator produced a one-instruction HALT, because
// `SHPA0`'s address happens to decode as one. That is a fact about the data,
// not about the decoder, and asserting anything of it would have been
// asserting nonsense. The image is still loaded into the window below, since
// that is where the hardware puts it.

#[test]
#[ignore = "needs CHILL65_CORPUS"]
fn real_display_lists_execute_and_terminate() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let ship_build = build_sd_ship_full(&c);
    let ship = ship_build.bytes(SD_SHIP_BASE, SD_SHIP_LEN);
    let program = build_sd_program(&c);
    let mem = vector_window(&ship, &program);

    println!(
        "\n{:<10} {:>6}  {:>7} {:>6} {:>6}  {}",
        "list", "addr", "steps", "segs", "depth", "stop"
    );

    let mut drew = 0usize;
    for name in ROM_LISTS.iter().chain(GLYPH_LISTS.iter()) {
        let addr = program
            .symbol(name)
            .unwrap_or_else(|| panic!("{name} is not in the program symbol table"));

        let r = run_list(&mem, addr);
        println!(
            "{name:<10} {addr:04X}    {:>7} {:>6} {:>6}  {:?}",
            r.steps,
            r.segments.len(),
            r.max_depth,
            r.stop
        );

        assert!(
            r.stop.is_clean(),
            "{name} at {addr:04X} did not end cleanly: {:?} after {} steps",
            r.stop,
            r.steps
        );
        assert!(
            r.max_depth <= STACK_DEPTH,
            "{name} nested {} levels; the hardware has {STACK_DEPTH}",
            r.max_depth
        );
        if MUST_DRAW.contains(name) {
            assert!(
                !r.segments.is_empty(),
                "{name} at {addr:04X} is a shape and drew nothing"
            );
            drew += 1;
        }
    }

    assert_eq!(drew, MUST_DRAW.len(), "every shape should have been exercised");
    println!();
}

/// The generator must not wander outside the shapes it is given.
///
/// A list that walked into vector RAM — which is empty here — would decode
/// zeros as long vectors forever and only show up as a budget fault. This
/// checks the stronger property directly: every instruction a ROM shape
/// executes is fetched from ROM.
#[test]
#[ignore = "needs CHILL65_CORPUS"]
fn rom_lists_stay_within_rom() {
    let Some(c) = corpus() else {
        eprintln!("CHILL65_CORPUS unset — skipping");
        return;
    };

    let ship_build = build_sd_ship_full(&c);
    let ship = ship_build.bytes(SD_SHIP_BASE, SD_SHIP_LEN);
    let program = build_sd_program(&c);
    let mem = vector_window(&ship, &program);

    // Word 0x400 is CPU 2800, the first ROM word.
    const FIRST_ROM_WORD: u16 = (SD_SHIP_BASE - VEC_BASE) / 2;

    for name in ROM_LISTS.iter().chain(GLYPH_LISTS.iter()) {
        let addr = program
            .symbol(name)
            .unwrap_or_else(|| panic!("{name} is not in the program symbol table"));

        let mut vg = Vg::new();
        vg.reset((addr - VEC_BASE) / 2);
        vg.intensity = 7;
        let mut segments = Vec::new();
        let mut lowest = u16::MAX;
        for _ in 0..BUDGET {
            lowest = lowest.min(vg.pc);
            if vg.step(&mem, &mut segments).is_some() {
                break;
            }
        }
        assert!(
            lowest >= FIRST_ROM_WORD,
            "{name} fetched word {lowest:03X} (CPU {:04X}), inside vector RAM",
            VEC_BASE + lowest * 2
        );
    }
}
