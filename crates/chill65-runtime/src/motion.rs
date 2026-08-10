//! Motion objects — the sprite hardware that draws the characters.
//!
//! Crystal Castles draws the castle and the crystals into the bitmap in main
//! RAM. Everything that moves — Bentley Bear and everything chasing him — is a
//! **motion object**, drawn by separate hardware that composites over the
//! bitmap through the colour-address arbitration in [`crate::video::cram_address`].
//!
//! Transcribed from `Arcade-CrystalCastles_MiSTer/rtl/`: `MotionObjectBuffer.v`,
//! `MotionObjectHorizontalControl.v`, `MotionObjectVerticalControl.v`,
//! `MotionObjectPictureRom.v`, `PositionControl.v`, and `WorkingRam.v:16,41`
//! for where the table lives and how it is presented.
//!
//! # The table
//!
//! `WorkingRam.v:16` addresses SRAM from the video side as
//! `{3'b111, BUF1BUF2n, HC[8:2]}` — an 11-bit *word* address. `BUF1BUF2n` is
//! OUT1 bit 7 (`MT.BSL`, `9F07`), so the table is one of two 256-byte halves:
//!
//! ```text
//! MT.BSL = 0   SRAM 0xE00-0xEFF   CPU 8E00-8EFF
//! MT.BSL = 1   SRAM 0xF00-0xFFF   CPU 8F00-8FFF
//! ```
//!
//! `HC[8:2]` steps one word every four pixel clocks, and `PositionControl.v`
//! loads a horizontal counter once every eight (`HC[2:0] == 101`). So each
//! object occupies **two words, four bytes**, and exactly **40 of them** are
//! scanned per line — `HC` runs 0..319, giving 40 slots. Entries 40 to 63 exist
//! in RAM and never draw.
//!
//! `WorkingRam.v:41` gives `SR = {ic6D, ic6B}` where `ic6B` holds even
//! addresses and `ic6D` odd, so within a word the low byte is even and the high
//! byte odd. Combined with which phase reads which word:
//!
//! ```text
//! byte 0   picture code      latched from SR[7:0]   (MotionObjectPictureRom.v:16-19)
//! byte 1   vertical position read as SR[15:8]       (MotionObjectVerticalControl.v:13)
//! byte 2   bit 7 = MPI       latched from SR[7]     (MotionObjectBuffer.v:14-18)
//! byte 3   horizontal position, loaded as SR[15:8]  (MotionObjectHorizontalControl.v)
//! ```
//!
//! # Vertical
//!
//! `MotionObjectVerticalControl.v:13-20`:
//!
//! ```verilog
//! wire [7:0] sum = VC + SR[15:8];
//! MATCHn <= ~(sum[7] & sum[6] & sum[5] & sum[4]);
//! q <= sum[3:0];
//! ```
//!
//! The object is on this line when `sum & 0xF0 == 0xF0` — sixteen lines tall,
//! since only sixteen values of `VC` put the sum in `F0..FF` — and `q` is the
//! row within it. A non-matching object forces every plane to `1111`
//! (`MotionObjectPictureRom.v:68-70`), which is colour 7: transparent. That is
//! how "no object here" is expressed, not by a separate enable.
//!
//! # The picture ROMs
//!
//! `addr = {picture, q ^ {4{PLAYER2}}, ~CK1 ^ PLAYER2}` — 13 bits, which is one
//! 2764. Each fetch yields three bit-planes of four pixels:
//!
//! ```verilog
//! nib3 = data_ic8D[3:0];   nib2 = data_ic8B[7:4];   nib1 = data_ic8B[3:0];
//! ```
//!
//! Note **only the low nibble of `8D` is used**; its high nibble is not wired.
//! Two fetches — the two values of the low address bit — make an object **8
//! wide by 16 tall at 3 bits per pixel**.
//!
//! `MotionObjectPictureRom.v:104` reads the shifters MSB-first when `PLAYER2`
//! is clear and LSB-first when it is set, and the row is XORed with `0xF` under
//! `PLAYER2` — so cocktail flip mirrors both axes.
//!
//! # The one-line delay
//!
//! There are two line buffers. `PositionControl.v` alternates them on `VC[0]`:
//! one is written with this line's objects while the other is read out and
//! blanked behind itself (`MotionObjectBuffer.v:20`, where the read side writes
//! `4'b1111` back). **A buffer filled while the counter reads `VC` is displayed
//! on line `VC + 1`** — see [`DISPLAY_DELAY_LINES`].
//!
//! # What this model does not reproduce
//!
//! It composites **once per frame from the final SRAM and OUT1 state**, which
//! is what the rest of the project's frame extraction does — `Machine` has no
//! scanline-level video model and `harness.md` §13.6 records why building one
//! is out of scope.
//!
//! And the **horizontal phase is a structural reading, not a cycle-exact one.**
//! The write counter takes `hpos` at `HC[2:0] == 101` while the first nibble
//! pair loads at `001`, and `CK1` is registered on a CPU-rate enable
//! (`PositionControl.v:15`), so where an object's first pixel lands relative to
//! `hpos` follows from cycle-level timing this model does not have. Objects are
//! drawn at `hpos..hpos+7`; if measurement against the oracles says otherwise,
//! the correction belongs here and is a constant.

use crate::video::WIDTH;

/// Objects scanned per line. `HC` runs 0..319 and a slot is eight counts.
pub const OBJECTS: usize = 40;

/// Bytes per object: two words of the shift register.
pub const OBJECT_BYTES: usize = 4;

/// The colour meaning "nothing here". Not a colour the hardware can draw.
pub const TRANSPARENT: u8 = 7;

/// Object height in scanlines — the sixteen values of `VC` that match.
pub const OBJECT_HEIGHT: u8 = 16;

/// Object width in pixels: two fetches of four.
pub const OBJECT_WIDTH: u8 = 8;

/// Lines between a buffer being filled and being displayed.
///
/// The buffers swap on `VC[0]`, so what is evaluated while the counter reads
/// `VC` appears on the line after. This is the single likeliest thing an oracle
/// calibration will want to change, which is why it is a named constant rather
/// than an offset buried in arithmetic.
pub const DISPLAY_DELAY_LINES: u8 = 1;

/// One pixel of a motion-object line buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pixel {
    /// `MV[2:0]`, the object colour. [`TRANSPARENT`] means no object.
    pub mv: u8,
    /// `MPI`, the priority bit — see [`crate::video::cram_address`].
    pub mpi: bool,
}

impl Pixel {
    /// The empty pixel: what a blanked line buffer holds.
    pub const NONE: Pixel = Pixel {
        mv: TRANSPARENT,
        mpi: true,
    };
}

/// Where the object table starts in SRAM, from `MT.BSL` — OUT1 bit 7.
pub fn table_base(out1: u8) -> usize {
    if out1 & 0x80 != 0 {
        0xF00
    } else {
        0xE00
    }
}

/// Evaluate the object table at vertical counter `vc`.
///
/// Returns the 256-pixel line buffer, which the hardware displays on line
/// `vc + `[`DISPLAY_DELAY_LINES`].
///
/// - `sram` is the machine's 4 KB SRAM, `8000-8FFF`.
/// - `mob_rom` is 16384 bytes: `136022-106.8d` then `136022-107.8b`.
/// - `out1` supplies `MT.BSL` (bit 7) and `PLAYER2` (bit 4).
pub fn render_line(sram: &[u8], mob_rom: &[u8], out1: u8, vc: u8) -> [Pixel; WIDTH] {
    let mut line = [Pixel::NONE; WIDTH];
    let base = table_base(out1);
    let player2 = out1 & 0x10 != 0;

    for slot in 0..OBJECTS {
        let entry = base + slot * OBJECT_BYTES;
        let picture = sram[entry];
        let vpos = sram[entry + 1];
        let mpi = sram[entry + 2] & 0x80 != 0;
        let hpos = sram[entry + 3];

        // MATCHn: on this line only while the sum's top nibble is all ones.
        let sum = vc.wrapping_add(vpos);
        if sum & 0xF0 != 0xF0 {
            continue;
        }
        // `q ^ {4{PLAYER2}}` — the flip mirrors vertically.
        let row = (sum & 0x0F) ^ if player2 { 0x0F } else { 0x00 };

        for half in 0..2u8 {
            // `~CK1 ^ PLAYER2`: the flip also swaps which four pixels come
            // first, which is what mirrors the object horizontally.
            let half_bit = half ^ u8::from(player2);
            let addr = ((picture as usize) << 5) | ((row as usize) << 1) | half_bit as usize;

            let plane3 = mob_rom[addr] & 0x0F; // ic8D, low nibble only
            let plane2 = mob_rom[0x2000 + addr] >> 4; // ic8B high
            let plane1 = mob_rom[0x2000 + addr] & 0x0F; // ic8B low

            for i in 0..4u8 {
                // MSB first, or LSB first under the flip.
                let b = if player2 { i } else { 3 - i };
                let mv = (((plane3 >> b) & 1) << 2)
                    | (((plane2 >> b) & 1) << 1)
                    | ((plane1 >> b) & 1);
                if mv == TRANSPARENT {
                    continue;
                }
                // The write counter is eight bits and simply wraps.
                let x = hpos.wrapping_add(half * 4 + i);
                line[x as usize] = Pixel { mv, mpi };
            }
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A ROM in which picture `p`, row `r`, half `h` is one chosen 3bpp
    /// pattern: four pixels, given MSB-first as colour values.
    fn rom_with(entries: &[(u8, u8, u8, [u8; 4])]) -> Vec<u8> {
        let mut rom = vec![0u8; 0x4000];
        // Colour 7 everywhere by default: 8D low nibble and both 8B nibbles all
        // ones is MV = 7, transparent.
        for a in 0..0x2000 {
            rom[a] = 0x0F;
            rom[0x2000 + a] = 0xFF;
        }
        for &(picture, row, half, pixels) in entries {
            let addr = ((picture as usize) << 5) | ((row as usize) << 1) | half as usize;
            let (mut p3, mut p2, mut p1) = (0u8, 0u8, 0u8);
            for (i, &mv) in pixels.iter().enumerate() {
                let b = 3 - i as u8;
                p3 |= ((mv >> 2) & 1) << b;
                p2 |= ((mv >> 1) & 1) << b;
                p1 |= (mv & 1) << b;
            }
            rom[addr] = p3;
            rom[0x2000 + addr] = (p2 << 4) | p1;
        }
        rom
    }

    /// One object in slot `slot` of the `MT.BSL = 0` table.
    fn sram_with(objects: &[(usize, u8, u8, bool, u8)]) -> Vec<u8> {
        let mut sram = vec![0u8; 0x1000];
        for &(slot, picture, vpos, mpi, hpos) in objects {
            let e = 0xE00 + slot * OBJECT_BYTES;
            sram[e] = picture;
            sram[e + 1] = vpos;
            sram[e + 2] = if mpi { 0x80 } else { 0x00 };
            sram[e + 3] = hpos;
        }
        sram
    }

    /// Every pixel that is not transparent, as (x, mv).
    fn drawn(line: &[Pixel; WIDTH]) -> Vec<(usize, u8)> {
        line.iter()
            .enumerate()
            .filter(|(_, p)| p.mv != TRANSPARENT)
            .map(|(x, p)| (x, p.mv))
            .collect()
    }

    #[test]
    fn an_object_is_eight_wide_and_lands_at_its_horizontal_position() {
        // vpos such that vc=0 gives sum 0xF0: the first row.
        let sram = sram_with(&[(0, 1, 0xF0, false, 100)]);
        let rom = rom_with(&[(1, 0, 0, [1, 2, 3, 4]), (1, 0, 1, [5, 6, 0, 1])]);
        let line = render_line(&sram, &rom, 0x00, 0);
        assert_eq!(
            drawn(&line),
            vec![
                (100, 1),
                (101, 2),
                (102, 3),
                (103, 4),
                (104, 5),
                (105, 6),
                (106, 0),
                (107, 1)
            ]
        );
    }

    #[test]
    fn colour_seven_is_transparent_and_leaves_the_buffer_alone() {
        let sram = sram_with(&[(0, 1, 0xF0, false, 10)]);
        // Middle two pixels transparent.
        let rom = rom_with(&[(1, 0, 0, [3, 7, 7, 4])]);
        let line = render_line(&sram, &rom, 0x00, 0);
        let hits: Vec<_> = drawn(&line).into_iter().filter(|(x, _)| *x < 14).collect();
        assert_eq!(hits, vec![(10, 3), (13, 4)]);
    }

    #[test]
    fn an_object_is_sixteen_lines_tall_and_selects_its_row_by_the_sum() {
        // vpos = 0xF0 puts row 0 at vc = 0, so rows 0..15 at vc 0..15.
        let sram = sram_with(&[(0, 1, 0xF0, false, 0)]);
        let rom = rom_with(&[
            (1, 0, 0, [1, 7, 7, 7]),
            (1, 5, 0, [2, 7, 7, 7]),
            (1, 15, 0, [3, 7, 7, 7]),
        ]);
        assert_eq!(drawn(&render_line(&sram, &rom, 0, 0))[0].1, 1, "row 0");
        assert_eq!(drawn(&render_line(&sram, &rom, 0, 5))[0].1, 2, "row 5");
        assert_eq!(drawn(&render_line(&sram, &rom, 0, 15))[0].1, 3, "row 15");
        // One line past the bottom: sum wraps to 0x00 and MATCHn is asserted.
        assert!(drawn(&render_line(&sram, &rom, 0, 16)).is_empty(), "row 16");
        // And a line before the top.
        assert!(drawn(&render_line(&sram, &rom, 0, 255)).is_empty(), "above");
    }

    #[test]
    fn an_object_wraps_rather_than_running_off_the_edge() {
        let sram = sram_with(&[(0, 1, 0xF0, false, 253)]);
        let rom = rom_with(&[(1, 0, 0, [1, 2, 3, 4]), (1, 0, 1, [5, 6, 1, 2])]);
        let mut xs: Vec<usize> = drawn(&render_line(&sram, &rom, 0, 0))
            .into_iter()
            .map(|(x, _)| x)
            .collect();
        xs.sort();
        assert_eq!(xs, vec![0, 1, 2, 3, 4, 253, 254, 255]);
    }

    #[test]
    fn exactly_forty_entries_are_scanned() {
        let rom = rom_with(&[(1, 0, 0, [1, 7, 7, 7])]);
        // Slot 39 is the last one the hardware reaches.
        let sram = sram_with(&[(39, 1, 0xF0, false, 60)]);
        assert_eq!(drawn(&render_line(&sram, &rom, 0, 0)), vec![(60, 1)]);
        // Slot 40 is in RAM and never drawn.
        let sram = sram_with(&[(40, 1, 0xF0, false, 60)]);
        assert!(drawn(&render_line(&sram, &rom, 0, 0)).is_empty());
    }

    #[test]
    fn the_table_half_follows_out1_bit_seven() {
        let rom = rom_with(&[(1, 0, 0, [1, 7, 7, 7])]);
        let mut sram = vec![0u8; 0x1000];
        // An object in the MT.BSL=1 table only.
        let e = 0xF00;
        sram[e] = 1;
        sram[e + 1] = 0xF0;
        sram[e + 3] = 77;

        assert!(
            drawn(&render_line(&sram, &rom, 0x00, 0)).is_empty(),
            "MT.BSL clear reads 8E00 and should see nothing"
        );
        assert_eq!(
            drawn(&render_line(&sram, &rom, 0x80, 0)),
            vec![(77, 1)],
            "MT.BSL set reads 8F00"
        );
    }

    #[test]
    fn a_later_entry_draws_over_an_earlier_one() {
        // Two objects at the same place; the higher slot wins, because the
        // hardware simply writes them in order into the same line buffer.
        let sram = sram_with(&[(0, 1, 0xF0, false, 50), (7, 2, 0xF0, false, 50)]);
        let rom = rom_with(&[(1, 0, 0, [1, 7, 7, 7]), (2, 0, 0, [4, 7, 7, 7])]);
        assert_eq!(drawn(&render_line(&sram, &rom, 0, 0)), vec![(50, 4)]);
    }

    #[test]
    fn the_priority_bit_travels_with_the_pixel() {
        let rom = rom_with(&[(1, 0, 0, [1, 7, 7, 7])]);
        for mpi in [false, true] {
            let sram = sram_with(&[(0, 1, 0xF0, mpi, 20)]);
            let line = render_line(&sram, &rom, 0, 0);
            assert_eq!(line[20].mpi, mpi, "MPI should reach the buffer");
            assert_eq!(line[20].mv, 1);
        }
        // And an untouched pixel keeps the blank value.
        assert_eq!(render_line(&sram_with(&[]), &rom, 0, 0)[20], Pixel::NONE);
    }

    #[test]
    fn player_two_mirrors_both_axes() {
        // Distinct pixels across the width, and a distinctive row.
        let sram = sram_with(&[(0, 1, 0xF0, false, 40)]);
        let rom = rom_with(&[
            (1, 0, 0, [1, 2, 3, 4]),
            (1, 0, 1, [5, 6, 1, 2]),
            // Row 15 is what row 0 becomes under the vertical flip.
            (1, 15, 0, [6, 6, 6, 6]),
            (1, 15, 1, [5, 5, 5, 5]),
        ]);

        let upright: Vec<u8> = drawn(&render_line(&sram, &rom, 0x00, 0))
            .into_iter()
            .map(|(_, mv)| mv)
            .collect();
        assert_eq!(upright, vec![1, 2, 3, 4, 5, 6, 1, 2]);

        // PLAYER2 (OUT1 bit 4) flips the row to 15 and reverses the pixels.
        let flipped: Vec<u8> = drawn(&render_line(&sram, &rom, 0x10, 0))
            .into_iter()
            .map(|(_, mv)| mv)
            .collect();
        assert_eq!(flipped, vec![5, 5, 5, 5, 6, 6, 6, 6]);
    }

    #[test]
    fn a_blank_table_draws_nothing() {
        let rom = rom_with(&[]);
        let line = render_line(&vec![0u8; 0x1000], &rom, 0, 0);
        assert!(line.iter().all(|p| *p == Pixel::NONE));
    }
}
