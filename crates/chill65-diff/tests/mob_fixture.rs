//! Sprite geometry, against the verilated core, with the game removed.
//!
//! ```text
//! cargo test -p chill65-diff --test mob_fixture -- --ignored --nocapture
//! ```
//!
//! # Why a fixture and not a trace
//!
//! Geometry cannot be calibrated on a recording of the game, because our
//! runtime and an oracle are not in the same *game state* during one. Measured
//! on `gameplay.trace`: at frame 600 our object table is parked at `F0` on
//! every line while MAME draws four characters, and at frame 960 both sides
//! have objects but different ones in different places, so adding our sprites
//! *raises* the differing-pixel count. No sprite model reconciles that, and the
//! comparison says nothing about geometry.
//!
//! `fixture/mob.MAC` removes the game from the question: an original program of
//! ours plants a known object table, never touches it again, and idles.
//! Whatever an implementation then draws is a function of the sprite hardware
//! alone.
//!
//! # Why the core and not MAME
//!
//! The simulation reads its seven devices as a flat file and audits none of
//! them, so it boots a program of ours. MAME checks its set against its own
//! CRC-32s and will not. Needing no corpus is a bonus: this test compares two
//! independent readings of the same RTL, and no game bytes take part.

use std::path::PathBuf;
use std::process::Command;

use chill65_diff::images::workspace_root;
use chill65_diff::mister;
use chill65_diff::trace::Trace;
use chill65_runtime::frame::run_frame;
use chill65_runtime::video::{cram_rgb, WIDTH};
use chill65_runtime::{Cpu, Machine};

/// The core blanks its leading columns at the port (`harness.md` §11), so they
/// carry no information either way.
const FIRST_COL: usize = 4;
const FRAMES: u32 = 12;

/// Solid `colour` across one picture row, as three bit-planes of four pixels.
fn plant(rom: &mut [u8], picture: u8, rows: &[(u8, u8)]) {
    for &(row, colour) in rows {
        for half in 0..2u8 {
            let a = ((picture as usize) << 5) | ((row as usize) << 1) | half as usize;
            rom[a] = if colour & 4 != 0 { 0x0F } else { 0 };
            rom[0x2000 + a] = ((if colour & 2 != 0 { 0x0F } else { 0 }) << 4)
                | if colour & 1 != 0 { 0x0F } else { 0 };
        }
    }
}

/// Picture ROMs for the fixture: transparent everywhere but three pictures.
fn picture_roms() -> Vec<u8> {
    let mut rom = vec![0u8; 0x4000];
    for a in 0..0x2000 {
        rom[a] = 0x0F; // 8D low nibble
        rom[0x2000 + a] = 0xFF; // 8B both nibbles
    }
    // 1: colour 1, with the top row 4 and the bottom row 2, so a vertical flip
    // or an off-by-one row would show.
    let mut rows: Vec<(u8, u8)> = (1..15).map(|r| (r, 1u8)).collect();
    rows.push((0, 4));
    rows.push((15, 2));
    plant(&mut rom, 1, &rows);
    // 2: colour 0, which the arbitration keeps opaque even under priority.
    plant(&mut rom, 2, &(0..16).map(|r| (r, 0u8)).collect::<Vec<_>>());
    // 3: colour 2.
    plant(&mut rom, 3, &(0..16).map(|r| (r, 2u8)).collect::<Vec<_>>());
    rom
}

/// Assemble `fixture/mob.MAC` through the real assembler CLI.
fn assemble() -> Vec<u8> {
    let root = workspace_root();
    let out = root.join("target/mob-fixture/prog.bin");
    std::fs::create_dir_all(out.parent().expect("parent")).expect("out dir");
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["run", "-q", "-p", "chill65-asm", "--", "mob.MAC"])
        .arg("-I")
        .arg(root.join("crates/chill65-diff/fixture"))
        .arg("-o")
        .arg(&out)
        .args(["--base", "A000", "--end", "FFFF"])
        .status()
        .expect("run chill65-asm");
    assert!(status.success(), "the fixture failed to assemble");
    std::fs::read(&out).expect("fixture image")
}

#[test]
#[ignore = "needs verilator on PATH"]
fn sprite_geometry_agrees_with_the_verilated_core() {
    if !mister::have_verilator() {
        eprintln!("verilator not on PATH — skipping");
        return;
    }
    let prog = assemble();
    let rom = picture_roms();

    // The download blob, in the core's own device order: 1f, 1h, 8d, 8b, 1k,
    // 1l, 1n. The castle-data bank is unused and zeroed.
    let mut blob = vec![0u8; 0x4000];
    blob.extend_from_slice(&rom);
    blob.extend_from_slice(&prog);
    let path: PathBuf = workspace_root().join("target/mister-sim/mob-fixture.bin");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("sim dir");
    std::fs::write(&path, &blob).expect("write blob");

    let capture = mister::capture_blob(&path, &Trace::idle(FRAMES as usize), FRAMES)
        .expect("the core should boot our program");

    let mut m = Machine::new();
    m.load_roms(&prog, &vec![0u8; 0x4000]).expect("load");
    m.load_motion_roms(&rom).expect("motion roms");
    let mut cpu = Cpu::new();
    cpu.reset(&mut m);
    for _ in 0..FRAMES {
        run_frame(&mut cpu, &mut m).expect("frame");
    }
    let ours: Vec<[u8; 3]> = m
        .framebuffer()
        .iter()
        .map(|&p| {
            let (r, g, b) = cram_rgb(m.video.cram[p as usize & 0x1F]);
            [r, g, b]
        })
        .collect();
    let theirs: Vec<[u8; 3]> = capture.frames[FRAMES as usize - 1]
        .chunks_exact(3)
        .map(|c| [c[0], c[1], c[2]])
        .collect();

    let mut differing = 0usize;
    let mut lit = 0usize;
    for (i, (a, b)) in ours.iter().zip(&theirs).enumerate() {
        let col = i % WIDTH;
        if col < FIRST_COL || col >= capture.emitted {
            continue;
        }
        if *a != [0, 0, 0] {
            lit += 1;
        }
        if a != b {
            differing += 1;
        }
    }

    eprintln!(
        "mob fixture: {lit} lit pixels over {} emitted columns, {differing} differing",
        capture.emitted
    );
    assert!(lit > 0, "the fixture drew nothing — it is proving nothing");
    assert_eq!(
        differing, 0,
        "sprite geometry disagrees with the core in {differing} pixels"
    );
}
