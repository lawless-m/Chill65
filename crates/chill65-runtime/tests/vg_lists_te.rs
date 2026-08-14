//! Run Tempest's real display lists through the vector generator.
//!
//! The counterpart of `vg_lists.rs`, and it exists for the same reason: the
//! Phase 1 assembler reproduces Tempest's image byte-identically (`gate_te`),
//! so every display list in the built image is **known-correct input**. A
//! disagreement can only be the decoder's fault, never the data's.
//!
//! What is new here is the colour generator. `VGMC.MAC` is byte-identical
//! between the two corpora, so the instruction set under test is the same one
//! Space Duel exercises; the difference is [`StatDecode::ColorSelect`], and
//! these lists are full of `CSTAT` words that only make sense under it.
//!
//! ```text
//! CHILL65_CORPUS=/path/to/tempest \
//!   cargo test -p chill65-runtime --test vg_lists_te -- --ignored --nocapture
//! ```

mod common;

use chill65_runtime::vg::{Segment, StatDecode, Stop, Vg, STACK_DEPTH, VEC_BASE, VEC_BYTES};

use common::{build_te_program, corpus, TeBuild, TE_VEC_ROM_BASE, TE_VEC_ROM_LEN};

/// How the board populates the generator's 8K window.
///
/// `ALCOMN.MAC:256-257` gives `VECRAM=2000` and `ROMSTART=3000`, so Tempest
/// splits the window in half where Space Duel cuts it in three:
///
/// ```text
/// 2000-2FFF   vector RAM    the CPU's display list; empty here
/// 3000-3FFF   vector ROM    from the twelve-module link
/// ```
fn vector_window(program: &TeBuild) -> Vec<u8> {
    let mut mem = vec![0u8; VEC_BYTES];
    let off = (TE_VEC_ROM_BASE - VEC_BASE) as usize;
    mem[off..off + TE_VEC_ROM_LEN].copy_from_slice(&program.bytes(TE_VEC_ROM_BASE, TE_VEC_ROM_LEN));
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
/// Intensity is latched bright first, for `vg_lists.rs`'s reason: these shapes
/// are called by game code that sets brightness beforehand, and most of their
/// vectors use `ZZ=1` — "use the latched value". Starting dark would render
/// them invisible for reasons having nothing to do with the decoder.
fn run_list(mem: &[u8], addr: u16) -> Run {
    assert!(
        (VEC_BASE..VEC_BASE + VEC_BYTES as u16).contains(&addr),
        "{addr:04X} is outside the vector window"
    );
    let mut vg = Vg::new();
    vg.stat_decode = StatDecode::ColorSelect;
    vg.reset((addr - VEC_BASE) / 2);
    vg.intensity = 0xF;
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

/// Character glyphs from `ANVGAN.MAC`.
///
/// One per character, each a run of `VCTR`s ending in `RTSL`
/// (`ANVGAN.MAC:34-352`), called by a `JSRL` the message code emits. `CHAR.`
/// is the blank and draws nothing, which is why it is executed but excluded
/// from [`MUST_DRAW`].
const GLYPH_LISTS: [&str; 6] = ["CHAR.A", "CHAR.E", "CHAR.T", "CHAR.Z", "CHAR.1", "CHAR."];

/// Shapes from the picture ROM, `ALVROM.MAC`.
const ROM_LISTS: [&str; 8] = [
    "HATCH",  // the well hatching
    "CHEKER", // checkerboard
    "VORBOX", // vortex box
    "EXPL1",  // explosion, first frame
    "STAR1",  // starfield
    "SPIRA1", // spiral
    "TANKP",  // the player's shape
    "BOKLIT", // the bookkeeping/literal block
];

/// Lists that are shapes and must therefore draw something.
const MUST_DRAW: [&str; 9] = [
    "CHAR.A", "CHAR.E", "CHAR.T", "CHAR.Z", "CHAR.1", "HATCH", "CHEKER", "VORBOX", "TANKP",
];

#[test]
#[ignore = "needs CHILL65_CORPUS pointing at a local source tree"]
fn real_display_lists_execute_and_terminate() {
    let Some(c) = corpus() else {
        return;
    };
    let program = build_te_program(&c);
    let mem = vector_window(&program);

    let mut ran = 0;
    for name in GLYPH_LISTS.iter().chain(ROM_LISTS.iter()) {
        let Some(addr) = program.symbol(name) else {
            panic!("{name} is not in the symbol table");
        };
        assert!(
            (TE_VEC_ROM_BASE..TE_VEC_ROM_BASE + TE_VEC_ROM_LEN as u16).contains(&addr),
            "{name} at {addr:04X} is outside the vector ROM"
        );

        let r = run_list(&mem, addr);
        eprintln!(
            "  {name:8} {addr:04X}  {:>5} steps  depth {}  {:>4} segments  {:?}",
            r.steps,
            r.max_depth,
            r.segments.len(),
            r.stop
        );

        assert!(
            matches!(r.stop, Stop::Returned | Stop::Halt),
            "{name} did not terminate cleanly: {:?}",
            r.stop
        );
        assert!(
            r.max_depth <= STACK_DEPTH,
            "{name} nested {} deep, past the hardware's {STACK_DEPTH}",
            r.max_depth
        );
        if MUST_DRAW.contains(name) {
            assert!(!r.segments.is_empty(), "{name} is a shape and drew nothing");
        }
        ran += 1;
    }
    assert_eq!(ran, GLYPH_LISTS.len() + ROM_LISTS.len());
}
