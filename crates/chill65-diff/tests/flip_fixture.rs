//! Cocktail flip geometry, measured on the verilated core.
//!
//! ```text
//! cargo test -p chill65-diff --test flip_fixture -- --ignored --nocapture
//! ```
//!
//! # The question
//!
//! The core's README carries one known issue: "In cocktail mode the player 2
//! upside down screen and sprites are not positioned correctly." Nothing we can
//! record reaches it — attract mode never sets `HW.FLV` — so `fixture/flip.MAC`
//! sets `PLAYER2` itself, half way through a run, on a scene that is otherwise
//! still. One capture then holds the same picture both ways up.
//!
//! # How a picture is turned into a number
//!
//! Mirroring maps a block spanning rows `[t,b]` to `[C-b, C-t]` for a constant
//! `C`, and the same in x. The fixture plants two bitmap markers and two motion
//! objects, so `C` can be read off the bitmap and off the objects separately.
//!
//! **If the hardware is right the two are the same number.** That comparison
//! needs no knowledge of the core's pixel pipeline, its blanked columns, or the
//! offset between an object's position byte and the column it lands on: every
//! such constant is common to the two orientations and cancels. It is also the
//! player's own complaint stated as arithmetic — "the sprites are in the wrong
//! place" *is* the two mirrors disagreeing.
//!
//! A whole-field alignment over the texture corroborates the bitmap's `C`
//! independently of the markers.
//!
//! # Why the fix is measured on a copy
//!
//! `Arcade-CrystalCastles_MiSTer` is reference material and is never modified.
//! The candidate fix is applied to a copy under `target/`, verilated
//! separately, and put through the identical measurement. The vendored tree is
//! read and never written — [`mister::build_at`] takes the RTL directory for
//! exactly this reason.

use std::path::{Path, PathBuf};
use std::process::Command;

use chill65_diff::images::workspace_root;
use chill65_diff::mister;
use chill65_diff::trace::Trace;
use chill65_runtime::video::{cram_rgb, HEIGHT, WIDTH};

/// The core blanks its leading columns at the port (`harness.md` §11), so they
/// carry no information either way.
const FIRST_COL: usize = 4;

/// Enough for the fixture to draw (about 26 frames of it), settle for eight,
/// flip, and then stand still long enough that the switch is unambiguous.
const FRAMES: u32 = 56;

/// A colour the fixture paints, given the byte it writes through the 9FA0
/// strip. See `flip.MAC`'s palette note for why only components 0, 2 and 7 are
/// used: they are the ones the simulation's linear scaling and `cram_rgb`'s
/// measured ladder agree on, so a colour can be predicted by either route.
fn palette(byte: u16) -> [u8; 3] {
    let (r, g, b) = cram_rgb(0x100 | byte);
    [r, g, b]
}

/// Bitmap pixel value 15, which the texture never produces.
const MARKER: u16 = 0xC0;
/// The object's body and its two end rows.
const OBJECT: [u16; 3] = [0x7F, 0x7D, 0x78];

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

/// Picture ROMs: transparent everywhere but picture 1, whose body is colour 1
/// with a colour-4 top row and a colour-2 bottom row — so the object's own
/// mirror is legible in the capture and not merely assumed.
fn picture_roms() -> Vec<u8> {
    let mut rom = vec![0u8; 0x4000];
    for a in 0..0x2000 {
        rom[a] = 0x0F;
        rom[0x2000 + a] = 0xFF;
    }
    let mut rows: Vec<(u8, u8)> = (1..15).map(|r| (r, 1u8)).collect();
    rows.push((0, 4));
    rows.push((15, 2));
    plant(&mut rom, 1, &rows);
    rom
}

fn assemble() -> Vec<u8> {
    let root = workspace_root();
    let out = root.join("target/flip-fixture/prog.bin");
    std::fs::create_dir_all(out.parent().expect("parent")).expect("out dir");
    let status = Command::new("cargo")
        .current_dir(&root)
        .args(["run", "-q", "-p", "chill65-asm", "--", "flip.MAC"])
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

/// One frame as pixel triples.
struct Picture {
    px: Vec<[u8; 3]>,
    emitted: usize,
}

impl Picture {
    fn at(&self, row: usize, col: usize) -> [u8; 3] {
        self.px[row * WIDTH + col]
    }

    /// Rows and columns worth reading: inside the emitted window.
    fn cols(&self) -> std::ops::Range<usize> {
        FIRST_COL..self.emitted
    }

    /// Every colour on show, with a count. Only ever used to say what a
    /// picture *does* hold when it does not hold what was looked for — a
    /// failure that reports "not found" and nothing else costs a whole run to
    /// diagnose, and a run here is a minute of verilator.
    fn census(&self) -> Vec<([u8; 3], usize)> {
        let mut seen: Vec<([u8; 3], usize)> = Vec::new();
        for row in 0..HEIGHT {
            for col in self.cols() {
                let px = self.at(row, col);
                match seen.iter_mut().find(|(c, _)| *c == px) {
                    Some((_, n)) => *n += 1,
                    None => seen.push((px, 1)),
                }
            }
        }
        seen.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        seen
    }

    /// The bounding box of every pixel matching one of `colours`, as
    /// `(top, bottom, left, right)` inclusive.
    fn bbox(&self, colours: &[[u8; 3]]) -> Option<(usize, usize, usize, usize)> {
        let mut b: Option<(usize, usize, usize, usize)> = None;
        for row in 0..HEIGHT {
            for col in self.cols() {
                if colours.contains(&self.at(row, col)) {
                    b = Some(match b {
                        None => (row, row, col, col),
                        Some((t, bo, l, r)) => (t.min(row), bo.max(row), l.min(col), r.max(col)),
                    });
                }
            }
        }
        b
    }
}

/// The mirror constant a block implies: `[t,b]` maps to `[C-b, C-t]`, so `C` is
/// `flipped.top + upright.bottom`, and equally `flipped.bottom + upright.top`.
/// Both are returned, because they agree only if the block kept its size —
/// which is itself worth seeing rather than assuming.
fn centre(up: (usize, usize), fl: (usize, usize)) -> (isize, isize) {
    (
        fl.0 as isize + up.1 as isize,
        fl.1 as isize + up.0 as isize,
    )
}

/// The (vertical, horizontal) mirror constants that best align the whole
/// textured field, by exact pixel match over a sampled grid.
///
/// Reported alongside the marker reading as a check on it: the markers are
/// eight pixels of a picture that is otherwise 232 rows of texture, and an
/// alignment that agrees with them cannot be an artefact of where they were put.
fn field_alignment(up: &Picture, fl: &Picture, cv: std::ops::Range<isize>, ch: std::ops::Range<isize>) -> (isize, isize, usize, usize) {
    let mut best = (0isize, 0isize, 0usize);
    let mut total = 0usize;
    for c_v in cv.clone() {
        for c_h in ch.clone() {
            let mut hits = 0usize;
            let mut seen = 0usize;
            for row in (0..HEIGHT).step_by(2) {
                let src_row = c_v - row as isize;
                if src_row < 0 || src_row >= HEIGHT as isize {
                    continue;
                }
                for col in fl.cols().step_by(2) {
                    let src_col = c_h - col as isize;
                    if src_col < FIRST_COL as isize || src_col >= fl.emitted as isize {
                        continue;
                    }
                    seen += 1;
                    if fl.at(row, col) == up.at(src_row as usize, src_col as usize) {
                        hits += 1;
                    }
                }
            }
            if hits > best.2 {
                best = (c_v, c_h, hits);
                total = seen;
            }
        }
    }
    (best.0, best.1, best.2, total)
}

/// Capture, split into the settled upright frame and the settled flipped one.
fn split(frames: &[Vec<u8>], emitted: usize) -> (Picture, Picture, usize) {
    let as_px = |f: &Vec<u8>| -> Vec<[u8; 3]> {
        f.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
    };
    // The fixture draws for a while (every frame differs), stands still, flips
    // once, and stands still again. The flip is therefore the LAST frame
    // boundary at which anything changed.
    let switch = (1..frames.len())
        .rev()
        .find(|&i| frames[i] != frames[i - 1])
        .expect("the picture never changed, so the flip never happened");
    let still_before = (1..switch).rev().take_while(|&i| frames[i] == frames[i - 1]).count();
    let still_after = (switch + 1..frames.len()).take_while(|&i| frames[i] == frames[i - 1]).count();
    assert!(
        still_before >= 4 && still_after >= 4,
        "the picture is not settled either side of the flip: {still_before} frames before, \
         {still_after} after — the capture is too short or the fixture is still drawing"
    );
    (
        Picture { px: as_px(&frames[switch - 1]), emitted },
        Picture { px: as_px(&frames[frames.len() - 1]), emitted },
        switch,
    )
}

/// What one run said, in the units the argument is conducted in.
struct Reading {
    /// The mirror constant the bitmap markers imply, vertical then horizontal.
    bitmap: (isize, isize),
    /// The same, from the motion objects.
    objects: (isize, isize),
    report: String,
}

/// A frame as a PPM, for looking at. Nothing measures these; they are here
/// because a number that says "23 lines" is worth being able to see.
fn write_ppm(p: &Picture, path: &Path) {
    let mut out = format!("P6\n{WIDTH} {HEIGHT}\n255\n").into_bytes();
    for px in &p.px {
        out.extend_from_slice(px);
    }
    std::fs::write(path, out).expect("write ppm");
}

/// Run the fixture on the simulation in `dir` and report what its flip does.
fn measure(dir: &Path, blob: &Path, label: &str, stem: &str) -> Reading {
    let capture = mister::capture_blob_in(dir, blob, &Trace::idle(FRAMES as usize), FRAMES)
        .expect("the core should boot our program");
    let (up, fl, switch) = split(&capture.frames, capture.emitted);

    let marker = [palette(MARKER)];
    let object: Vec<[u8; 3]> = OBJECT.iter().map(|&b| palette(b)).collect();

    let find = |p: &Picture, what: &str, colours: &[[u8; 3]]| {
        p.bbox(colours).unwrap_or_else(|| {
            let census: Vec<_> = p.census().into_iter().take(8).collect();
            panic!(
                "no {what} in the {label} {} frame. Looked for {colours:?}; the eight \
                 commonest colours on show are {census:?}",
                if std::ptr::eq(p, &up) { "upright" } else { "flipped" }
            )
        })
    };
    let mb_up = find(&up, "marker", &marker);
    let mb_fl = find(&fl, "marker", &marker);
    let ob_up = find(&up, "object", &object);
    let ob_fl = find(&fl, "object", &object);

    // Both markers together, and both objects together, span the field; their
    // outer box mirrors exactly as one block does.
    let (bmp_v, bmp_v2) = centre((mb_up.0, mb_up.1), (mb_fl.0, mb_fl.1));
    let (bmp_h, bmp_h2) = centre((mb_up.2, mb_up.3), (mb_fl.2, mb_fl.3));
    let (obj_v, obj_v2) = centre((ob_up.0, ob_up.1), (ob_fl.0, ob_fl.1));
    let (obj_h, obj_h2) = centre((ob_up.2, ob_up.3), (ob_fl.2, ob_fl.3));

    let (fv, fh, hits, seen) = field_alignment(&up, &fl, 180..300, 200..300);

    let mut s = String::new();
    s.push_str(&format!("--- {label}\n"));
    s.push_str(&format!("flip took effect at frame {switch} of {FRAMES}\n"));
    s.push_str(&format!(
        "markers  upright rows {:>3}-{:<3} cols {:>3}-{:<3}   flipped rows {:>3}-{:<3} cols {:>3}-{:<3}\n",
        mb_up.0, mb_up.1, mb_up.2, mb_up.3, mb_fl.0, mb_fl.1, mb_fl.2, mb_fl.3
    ));
    s.push_str(&format!(
        "objects  upright rows {:>3}-{:<3} cols {:>3}-{:<3}   flipped rows {:>3}-{:<3} cols {:>3}-{:<3}\n",
        ob_up.0, ob_up.1, ob_up.2, ob_up.3, ob_fl.0, ob_fl.1, ob_fl.2, ob_fl.3
    ));
    s.push_str(&format!(
        "mirror constant   bitmap  vertical {bmp_v}/{bmp_v2}  horizontal {bmp_h}/{bmp_h2}\n"
    ));
    s.push_str(&format!(
        "mirror constant   objects vertical {obj_v}/{obj_v2}  horizontal {obj_h}/{obj_h2}\n"
    ));
    s.push_str(&format!(
        "disagreement      vertical {} lines, horizontal {} pixels\n",
        bmp_v - obj_v,
        bmp_h - obj_h
    ));
    s.push_str(&format!(
        "whole-field best alignment: vertical {fv}, horizontal {fh}, {hits}/{seen} pixels \
         ({:.1}%)\n",
        100.0 * hits as f64 / seen.max(1) as f64
    ));

    let dir = workspace_root().join("target/flip-fixture");
    write_ppm(&up, &dir.join(format!("{stem}-upright.ppm")));
    write_ppm(&fl, &dir.join(format!("{stem}-flipped.ppm")));

    // The field alignment is a second route to the bitmap's constant, over the
    // texture rather than over eight pixels of marker. If the two ever part
    // company the markers are being read wrong.
    assert_eq!(
        (fv, fh),
        (bmp_v, bmp_h),
        "{label}: the markers and the textured field disagree about the mirror"
    );

    Reading {
        bitmap: (bmp_v, bmp_h),
        objects: (obj_v, obj_h),
        report: s,
    }
}

/// Write a copy of the core's RTL with the vertical reload changed, and return
/// the directory to verilate.
///
/// The change is stated here as the exact text it replaces, so this test fails
/// loudly rather than silently measuring the unpatched core if the vendored
/// source ever moves under it.
fn patched_core() -> PathBuf {
    let src = mister::core_dir();
    let dst = workspace_root().join("target/mister-flipfix");
    let rtl = dst.join("rtl");
    std::fs::create_dir_all(&rtl).expect("copy dir");
    for entry in std::fs::read_dir(src.join("rtl")).expect("read rtl") {
        let entry = entry.expect("entry");
        if entry.path().is_file() {
            std::fs::copy(entry.path(), rtl.join(entry.file_name())).expect("copy file");
        }
    }
    let pokey = rtl.join("Pokey");
    std::fs::create_dir_all(&pokey).expect("pokey dir");
    for entry in std::fs::read_dir(src.join("rtl/Pokey")).expect("read pokey") {
        let entry = entry.expect("entry");
        if entry.path().is_file() {
            std::fs::copy(entry.path(), pokey.join(entry.file_name())).expect("copy file");
        }
    }

    let path = rtl.join("CCastles.v");
    let text = std::fs::read_to_string(&path).expect("CCastles.v");
    let before = "      if (VBLANK)\n         vi <= vr;";
    assert!(
        text.contains(before),
        "the vertical reload in CCastles.v is not where this test left it"
    );
    // Counting down from the same value re-walks the rows the upright picture
    // started at; the mirror needs the far end of the window. 231 is the
    // visible height less one.
    let after = "      if (VBLANK)\n         vi <= PLAYER2 ? vr + 8'd231 : vr;";
    std::fs::write(&path, text.replace(before, after)).expect("write patched CCastles.v");
    dst
}

#[test]
#[ignore = "needs verilator on PATH"]
fn the_cocktail_flip_puts_the_bitmap_and_the_objects_in_the_same_place() {
    if !mister::have_verilator() {
        eprintln!("verilator not on PATH — skipping");
        return;
    }
    let prog = assemble();
    let mut blob = vec![0u8; 0x4000];
    blob.extend_from_slice(&picture_roms());
    blob.extend_from_slice(&prog);

    let stock_dir = workspace_root().join("target/mister-sim");
    mister::build().expect("build the stock simulation");
    let blob_path = stock_dir.join("flip-fixture.bin");
    std::fs::create_dir_all(&stock_dir).expect("sim dir");
    std::fs::write(&blob_path, &blob).expect("write blob");

    let stock = measure(&stock_dir, &blob_path, "the core as it stands", "stock");

    let fix_dir = workspace_root().join("target/mister-flipfix-sim");
    mister::build_at(&patched_core(), &fix_dir).expect("build the patched simulation");
    let fixed = measure(
        &fix_dir,
        &blob_path,
        "the same core, vertical reload changed",
        "fixed",
    );

    let report = format!(
        "chill65-diff: cocktail flip geometry\n\
         fixture: crates/chill65-diff/fixture/flip.MAC\n\
         core:    Arcade-CrystalCastles_MiSTer, verilated; the second run from an\n\
         \x20        unmodified copy with one line of CCastles.v changed\n\n\
         {}\n{}",
        stock.report, fixed.report
    );
    eprintln!("{report}");
    let path = workspace_root().join("target/flip-fixture/report.txt");
    std::fs::write(&path, &report).expect("write report");
    eprintln!("wrote {}", path.display());

    // All four constants below are measured, and every one of them was
    // predicted from the RTL before the fixture was run.

    // The objects are the reference: their mirror is the game's own arithmetic
    // (EN.PMV) plus the object pipeline, and the flip bit does not touch it.
    assert_eq!(stock.objects, fixed.objects, "the fix moved the objects");
    assert_eq!(
        stock.objects,
        (233, 259),
        "the objects no longer mirror where they did"
    );

    // A 180-degree turn maps the 232 visible lines onto themselves, so the
    // vertical constant a correct bitmap must show is 231. The core shows 256:
    // it counts down from the row the upright picture STARTED at, so the
    // flipped picture is 25 rows adrift and the row floor smears the 25 lines
    // that fall off the bottom of the count.
    assert_eq!(
        stock.bitmap.0, 256,
        "the bitmap's vertical mirror is not where it was measured"
    );
    assert_eq!(
        fixed.bitmap.0, 231,
        "the reload change did not put the bitmap's vertical mirror at 231"
    );

    // What the player sees: bitmap against objects, before and after.
    assert_eq!(stock.bitmap.0 - stock.objects.0, 23);
    assert_eq!(fixed.bitmap.0 - fixed.objects.0, -2);

    // Horizontal is untouched by the change and stays four pixels out — twice
    // the two-column phase difference between the bitmap scan and the object
    // pipeline. Upright there is nothing to see it against; the flip doubles it
    // into view. This is the core's own outstanding question, and it is not
    // what the reload change is for.
    assert_eq!(stock.bitmap.1, 255);
    assert_eq!(fixed.bitmap.1, 255);
    assert_eq!(fixed.bitmap.1 - fixed.objects.1, -4);
}
