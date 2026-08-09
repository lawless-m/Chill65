//! The bitmap hardware as the CPU sees it.
//!
//! Everything here is transcribed from `Arcade-CrystalCastles_MiSTer/rtl/`.
//! The CPU-facing behaviour is fully determinate from the RTL; the parts that
//! are not are marked UNVERIFIED here and in `hardware.md` §5, because a guess
//! recorded as fact would poison Phase 3's differential harness.
//!
//! # The coordinate window
//!
//! Three addresses in zero page are hardware, not RAM
//! (`AddresDecoders.v:72-74`):
//!
//! ```text
//! XCOORDn = ~(BA == 16'h0000 & ~BRWn & ce2Hd)   // write only
//! YCOORDn = ~(BA == 16'h0001 & ~BRWn & ce2Hd)   // write only
//! BITMDn  = ~(BA == 16'h0002)                   // read AND write
//! ```
//!
//! Note what the `~BRWn` qualifier does and does not cover. `0000` and `0001`
//! latch a coordinate **only on write**; reading them is an ordinary RAM read.
//! `0002` has no such qualifier, so *any* access — read or write — is a bitmap
//! access. That asymmetry drives most of the code below.
//!
//! # Address formation
//!
//! `AutoIncrement.v:16`:
//!
//! ```text
//! assign DRBA = ~BITMDn ? {yCoord,xCoord[7:1]} : BA[14:0];
//! ```
//!
//! So the bitmap byte is at `yCoord * 128 + xCoord / 2`: 128 bytes per row,
//! two pixels per byte at 4bpp, 256 pixels across, 256 rows — exactly 32768
//! bytes. The bitmap *is* `0000-7FFF` in its entirety. Nothing is reserved for
//! it; the CPU's zero page and stack are simply the top-left of the picture.
//!
//! # Which nibble
//!
//! `PIXA = xCoord[0]` (`AutoIncrement.v:18`), and `DynamicRam.v:63`:
//!
//! ```text
//! dram_to_cpu = ~BITMDn ? { PIXA ? cpu_read[7:4] : cpu_read[3:0], 4'b000 } : cpu_read;
//! ```
//!
//! **Even x reads the low nibble, odd x the high nibble**, and the pixel is
//! returned in the *high* four bits of the byte the CPU sees, with the low four
//! zeroed. The write path agrees (`DynamicRam.v:40`):
//!
//! ```text
//! bmw_to_dram = PIXA ? { BD[7:4], cpu_read[3:0] } : { cpu_read[7:4], BD[7:4]};
//! ```
//!
//! Both cases take the pixel from `BD[7:4]` and preserve the neighbouring
//! pixel — a read-modify-write in hardware. **The low nibble of what the CPU
//! writes is discarded.**
//!
//! # Writes below row 32 are dropped
//!
//! `DynamicRam.v:42`:
//!
//! ```text
//! WE = ~BRWn & ce2Hd3 & ((BITMDn & ~DRAMn) | (~BITMDn & DRBA[14:12] != 3'b000));
//! ```
//!
//! A write *through the window* only lands when `DRBA[14:12]` is non-zero —
//! that is, when the target is at or above `0x1000`, which means `yCoord >= 32`.
//! Rows 0–31 are silently unwritable this way, protecting zero page, the stack
//! and the game's working RAM from the drawing routines. A direct store to the
//! same address has no such restriction: the first term covers ordinary DRAM
//! writes. Reads through the window are not restricted either — only writes.
//!
//! # Auto-increment
//!
//! `AutoIncrement.v:27` and `:42`. Any access to `0002` steps the coordinates,
//! under the control of four OUT1 bits, all active-low:
//!
//! | OUT1 bit | Address | Signal   | Meaning                        |
//! |----------|---------|----------|--------------------------------|
//! | 0        | `9F00`  | `AXn`    | 0 enables X auto-step          |
//! | 1        | `9F01`  | `AYn`    | 0 enables Y auto-step          |
//! | 2        | `9F02`  | `XINCn`  | 0 increments, 1 decrements X   |
//! | 3        | `9F03`  | `YINCn`  | 0 increments, 1 decrements Y   |
//!
//! matching the game's own equates in `CG.MAC:77-80` ("autoinc x, 0 on, FF
//! off" / "xinc, 0 inc, FF dec"). Both axes can step on the same access.

/// Visible width in pixels. `HBLANK0 = hcount[8]` (`SyncChain.v:44`), so
/// hcount 0–255 is active and 256–319 is blanked.
pub const WIDTH: usize = 256;

/// Visible height in scanlines. `VBLANK` is asserted for vcount 0–23 and clear
/// for 24–255 (`SyncChain.v:45`), giving 232 active lines.
pub const HEIGHT: usize = 232;

/// Bytes per bitmap row — 256 pixels at two per byte.
pub const ROW_BYTES: usize = 128;

/// The vertical scroll floor. `vs = vi <= 8'h18 ? 8'h18 : vi` (`CCastles.v:201`,
/// commented "limit range to 0x18..0xFF"), and the vertical scroll register
/// resets to this value rather than to zero (`CCastles.v:180`).
pub const VSCROLL_FLOOR: u8 = 0x18;

pub struct Video {
    /// `0000`. Write-only from the CPU's point of view.
    pub xcoord: u8,
    /// `0001`. Write-only.
    pub ycoord: u8,

    /// Colour RAM: 32 entries of **nine** bits.
    ///
    /// The ninth bit does not come from the data bus. `ColorMemory.v:29` writes
    /// `din = {BA[5], BD}` at `addr = BA[4:0]`, so address bit 5 supplies the
    /// top bit while the data bus supplies the low eight — one write sets all
    /// nine. A byte-wide model would silently lose a bit of every colour.
    ///
    /// The stored bits are read back as `o = {~rbg[8:6], ~rbg[2:0], ~rbg[5:3]}`
    /// (`ColorMemory.v:33`): bits 8–6 are red, 5–3 are **blue**, 2–0 are
    /// **green** — the two are transposed on the way out — and every component
    /// is active-low. Stored here exactly as written, without that transform.
    pub cram: [u16; 32],

    /// Horizontal scroll, loaded by `HSLD` (`9C80`). Resets to 0.
    pub hscroll: u8,
    /// Vertical scroll, loaded by `VSLD` (`9D00`). Resets to [`VSCROLL_FLOOR`].
    pub vscroll: u8,
}

impl Default for Video {
    fn default() -> Self {
        Self::new()
    }
}

impl Video {
    pub fn new() -> Self {
        Video {
            xcoord: 0,
            ycoord: 0,
            cram: [0; 32],
            hscroll: 0,
            vscroll: VSCROLL_FLOOR,
        }
    }

    /// The bitmap byte the coordinate window currently addresses:
    /// `{yCoord, xCoord[7:1]}`.
    #[inline]
    pub fn window_addr(&self) -> u16 {
        ((self.ycoord as u16) << 7) | (self.xcoord >> 1) as u16
    }

    /// `PIXA` — false selects the low nibble, true the high nibble.
    #[inline]
    pub fn pixa(&self) -> bool {
        self.xcoord & 1 != 0
    }

    /// Whether a write through the window at the current coordinate reaches
    /// memory: `DRBA[14:12] != 0`, i.e. row 32 and above.
    #[inline]
    pub fn window_write_enabled(&self) -> bool {
        self.window_addr() & 0x7000 != 0
    }

    /// Read the pixel under the window, in the CPU's format: value in the high
    /// nibble, low nibble zero.
    pub fn window_read(&self, ram: &[u8]) -> u8 {
        let byte = ram[self.window_addr() as usize];
        let nibble = if self.pixa() { byte >> 4 } else { byte & 0x0F };
        nibble << 4
    }

    /// Write the pixel under the window. The value comes from the high nibble
    /// of `value`; the neighbouring pixel in the same byte is preserved.
    pub fn window_write(&self, ram: &mut [u8], value: u8) {
        if !self.window_write_enabled() {
            return;
        }
        let addr = self.window_addr() as usize;
        let old = ram[addr];
        ram[addr] = if self.pixa() {
            (value & 0xF0) | (old & 0x0F)
        } else {
            (old & 0xF0) | (value >> 4)
        };
    }

    /// Step the coordinates after an access to `0002`, per the OUT1 latch.
    pub fn auto_step(&mut self, out1: u8) {
        // Active low throughout: a clear bit enables, a clear direction bit
        // means increment.
        if out1 & 0x01 == 0 {
            self.xcoord = if out1 & 0x04 == 0 {
                self.xcoord.wrapping_add(1)
            } else {
                self.xcoord.wrapping_sub(1)
            };
        }
        if out1 & 0x02 == 0 {
            self.ycoord = if out1 & 0x08 == 0 {
                self.ycoord.wrapping_add(1)
            } else {
                self.ycoord.wrapping_sub(1)
            };
        }
    }

    /// Write a colour RAM entry. The strip is at `9F80`; the entry is the low
    /// five address bits and bit 5 of the address supplies bit 8 of the value.
    pub fn cram_write(&mut self, addr: u16, value: u8) {
        let entry = (addr & 0x1F) as usize;
        let hi = (addr >> 5) & 1;
        self.cram[entry] = (hi << 8) | value as u16;
    }

    /// The bitmap row displayed on visible scanline `line`.
    ///
    /// `vi` is reloaded from the scroll register during vblank and steps once
    /// per line (`CCastles.v:184-196`), then `vs = vi <= 0x18 ? 0x18 : vi`
    /// clamps it. The clamp is not cosmetic: because `vi` is eight bits, a
    /// large scroll value wraps past 255 and the bottom of the screen repeats
    /// row 0x18 rather than showing rows 0–23.
    #[inline]
    pub fn row_for_line(&self, line: usize) -> u8 {
        let vi = self.vscroll.wrapping_add(line as u8);
        if vi <= VSCROLL_FLOOR {
            VSCROLL_FLOOR
        } else {
            vi
        }
    }

    /// Extract the visible picture as one byte per pixel, each 0–15, row-major,
    /// `WIDTH * HEIGHT` long.
    ///
    /// **This does not clear anything.** The bitmap is never cleared per frame
    /// — only between levels — so the machine's RAM is the picture, and any
    /// implicit zeroing here would destroy the game's state rather than merely
    /// producing a wrong image.
    ///
    /// Horizontal scroll is a rotation within the row: `hs` is seeded by `HSLD`
    /// and steps once per visible pixel, and 256 steps of a 7-bit byte index
    /// cover exactly one 128-byte row.
    pub fn framebuffer(&self, ram: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; WIDTH * HEIGHT];
        for line in 0..HEIGHT {
            let row = self.row_for_line(line) as usize;
            let base = row * ROW_BYTES;
            for px in 0..WIDTH {
                let x = self.hscroll.wrapping_add(px as u8);
                let byte = ram[base + (x >> 1) as usize];
                // Same parity rule as the CPU sees; see `nibble_parity` below.
                out[line * WIDTH + px] = if x & 1 != 0 { byte >> 4 } else { byte & 0x0F };
            }
        }
        out
    }
}

/// The CRAM entry a bitmap pixel selects when no motion object covers it.
///
/// **Derived from `ColorMemory.v:22-31`, not measured.** The colour address is
/// arbitrated between the bitmap and the motion objects:
///
/// ```verilog
/// sel = (~MV[2] & ~MV[1] & ~MV[0]) | (~MV[2] & ~MPI) | (~MV[2] & ~BIT[3]) | ...
/// A4  = (MV[0] & MPI & BIT[3]) | ... | (MV[2] & MV[1] & MV[0]);
/// A3  = sel ? MPI : BIT[3];   A2 = sel ? MV[2] : BIT[2];  ...
/// ```
///
/// With `MV = 3'b111` — the "no object here" encoding, since every `sel` term
/// contains some `~MV[i]` — `sel` collapses to 0, `A4` is driven high by its
/// last term, and `A3:A0` pass `BIT[3:0]` straight through. So the address is
/// `{1, pixel}`: **entries 16–31**.
///
/// Motion objects are not modelled at all, so the arbitration above is not
/// implemented. If an extracted picture ever comes out in the wrong colours,
/// this constant is the first thing to doubt.
pub const BITMAP_CRAM_BASE: usize = 16;

/// Decode a stored colour RAM entry to 8-bit-per-channel RGB.
///
/// `ColorMemory.v:33`: `o = { ~rbg[8:6], ~rbg[2:0], ~rbg[5:3] }`. Three things
/// to get wrong here, all of which this encodes: the components are
/// **active-low**, green comes from the **bottom** three bits, and blue from
/// the **middle** three — the two are transposed relative to the obvious
/// reading of "rbg".
pub fn cram_rgb(entry: u16) -> (u8, u8, u8) {
    let r = (!(entry >> 6) & 0x7) as u8;
    let g = (!entry & 0x7) as u8;
    let b = (!(entry >> 3) & 0x7) as u8;
    let scale = |v: u8| ((v as u16 * 255) / 7) as u8;
    (scale(r), scale(g), scale(b))
}

/// FNV-1a, 64-bit. Hand-rolled: the runtime core takes no third-party crates,
/// and a frame identity only needs to be stable and well-mixed, not secure.
pub fn fnv1a(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET_BASIS;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ram() -> Vec<u8> {
        vec![0u8; 0x8000]
    }

    #[test]
    fn the_window_addresses_128_bytes_per_row() {
        let mut v = Video::new();
        v.ycoord = 0;
        v.xcoord = 0;
        assert_eq!(v.window_addr(), 0x0000);
        v.xcoord = 255;
        assert_eq!(v.window_addr(), 127, "two pixels share a byte");
        v.ycoord = 1;
        v.xcoord = 0;
        assert_eq!(v.window_addr(), 128, "one row on");
        v.ycoord = 255;
        v.xcoord = 255;
        assert_eq!(v.window_addr(), 0x7FFF, "the bitmap is exactly 0000-7FFF");
    }

    #[test]
    fn even_x_is_the_low_nibble_and_odd_x_the_high_one() {
        let mut r = ram();
        let addr = 40 * ROW_BYTES + 5;
        r[addr] = 0xA3;
        let mut v = Video::new();
        v.ycoord = 40;

        v.xcoord = 10; // even -> low nibble
        assert_eq!(v.window_addr() as usize, addr);
        assert_eq!(v.window_read(&r), 0x30, "value arrives in the HIGH nibble");

        v.xcoord = 11; // odd -> high nibble
        assert_eq!(v.window_addr() as usize, addr, "same byte");
        assert_eq!(v.window_read(&r), 0xA0);
    }

    #[test]
    fn writing_preserves_the_neighbouring_pixel() {
        // The hardware read-modify-writes; a naive byte store would wipe the
        // pixel next door.
        let mut r = ram();
        let addr = 40 * ROW_BYTES + 5;
        r[addr] = 0xA3;
        let mut v = Video::new();
        v.ycoord = 40;

        v.xcoord = 10;
        v.window_write(&mut r, 0x70);
        assert_eq!(r[addr], 0xA7, "low nibble replaced, high untouched");

        v.xcoord = 11;
        v.window_write(&mut r, 0x50);
        assert_eq!(r[addr], 0x57, "high nibble replaced, low untouched");
    }

    #[test]
    fn only_the_high_nibble_of_the_written_byte_is_used() {
        let mut r = ram();
        let mut v = Video::new();
        v.ycoord = 40;
        v.xcoord = 0;
        v.window_write(&mut r, 0x5F);
        assert_eq!(
            r[40 * ROW_BYTES],
            0x05,
            "BD[7:4] is the pixel; the low nibble of the store is discarded"
        );
    }

    #[test]
    fn window_writes_below_row_32_are_dropped() {
        // DynamicRam.v:42 gates window writes on DRBA[14:12] != 0. This keeps
        // the drawing routines from scribbling over zero page and the stack.
        let mut r = ram();
        let mut v = Video::new();

        for y in [0u8, 1, 16, 31] {
            v.ycoord = y;
            v.xcoord = 0;
            assert!(!v.window_write_enabled(), "row {y} must be protected");
            v.window_write(&mut r, 0xF0);
            assert_eq!(r[v.window_addr() as usize], 0, "row {y} unchanged");
        }

        v.ycoord = 32;
        v.xcoord = 0;
        assert!(v.window_write_enabled());
        v.window_write(&mut r, 0xF0);
        assert_eq!(r[32 * ROW_BYTES], 0x0F, "row 32 is the first writable one");
    }

    #[test]
    fn reads_below_row_32_are_not_restricted() {
        // Only writes are gated. The window can read anything.
        let mut r = ram();
        r[0] = 0x0C;
        let mut v = Video::new();
        v.ycoord = 0;
        v.xcoord = 0;
        assert_eq!(v.window_read(&r), 0xC0);
    }

    #[test]
    fn auto_step_follows_the_out1_latch() {
        let mut v = Video::new();
        v.xcoord = 100;
        v.ycoord = 100;

        // Reset state: all bits clear, so both axes enabled and incrementing.
        v.auto_step(0x00);
        assert_eq!((v.xcoord, v.ycoord), (101, 101));

        // Bit 2 set: X decrements. Bit 3 set: Y decrements.
        v.auto_step(0x0C);
        assert_eq!((v.xcoord, v.ycoord), (100, 100));

        // Bit 0 set disables X; bit 1 set disables Y.
        v.auto_step(0x03);
        assert_eq!((v.xcoord, v.ycoord), (100, 100), "both disabled");

        // X only.
        v.auto_step(0x02);
        assert_eq!((v.xcoord, v.ycoord), (101, 100));
    }

    #[test]
    fn coordinates_wrap_rather_than_overflow() {
        let mut v = Video::new();
        v.xcoord = 255;
        v.ycoord = 0;
        v.auto_step(0x0A); // X increments, Y disabled
        assert_eq!(v.xcoord, 0);
        v.xcoord = 0;
        v.auto_step(0x0E); // X decrements, Y disabled
        assert_eq!(v.xcoord, 255);
    }

    #[test]
    fn cram_takes_its_ninth_bit_from_address_bit_5() {
        let mut v = Video::new();
        v.cram_write(0x9F80, 0xFF);
        assert_eq!(v.cram[0], 0x0FF, "BA[5] clear");
        v.cram_write(0x9FA0, 0xFF);
        assert_eq!(v.cram[0], 0x1FF, "BA[5] set — same entry, ninth bit on");
        // Thirty-two entries, and the strip mirrors above them.
        v.cram_write(0x9F9F, 0x12);
        assert_eq!(v.cram[31], 0x012);
    }

    #[test]
    fn vertical_scroll_clamps_at_the_floor() {
        let mut v = Video::new();
        assert_eq!(v.vscroll, VSCROLL_FLOOR, "resets to 0x18, not to zero");
        assert_eq!(v.row_for_line(0), 0x18);
        assert_eq!(v.row_for_line(1), 0x19);
        assert_eq!(v.row_for_line(HEIGHT - 1), 0xFF, "0x18 + 231 = 255");

        // A large scroll wraps and the tail clamps rather than showing rows 0-23.
        v.vscroll = 0xF0;
        assert_eq!(v.row_for_line(0), 0xF0);
        assert_eq!(v.row_for_line(16), VSCROLL_FLOOR, "wrapped to 0, clamped up");
    }

    #[test]
    fn the_framebuffer_is_the_visible_window_over_raw_ram() {
        let mut r = ram();
        // Row 0x18 is the first displayed line.
        r[0x18 * ROW_BYTES] = 0x9C;
        let v = Video::new();
        let fb = v.framebuffer(&r);
        assert_eq!(fb.len(), WIDTH * HEIGHT);
        assert_eq!(fb[0], 0x0C, "x=0 is even -> low nibble");
        assert_eq!(fb[1], 0x09, "x=1 is odd -> high nibble");
    }

    #[test]
    fn horizontal_scroll_rotates_within_the_row() {
        let mut r = ram();
        r[0x18 * ROW_BYTES] = 0x0C; // pixel at x=0
        let mut v = Video::new();
        v.hscroll = 2;
        let fb = v.framebuffer(&r);
        assert_eq!(fb[0], 0, "scrolled past it");
        assert_eq!(fb[WIDTH - 2], 0x0C, "it reappears at the far end");
    }

    #[test]
    fn the_framebuffer_never_clears_ram() {
        // The bitmap is not cleared per frame, only between levels. Extraction
        // must be pure observation.
        let mut r = ram();
        for (i, b) in r.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let before = r.clone();
        let v = Video::new();
        let _ = v.framebuffer(&r);
        assert_eq!(r, before);
    }

    #[test]
    fn the_frame_hash_tracks_the_picture() {
        let mut r = ram();
        let v = Video::new();
        let a = fnv1a(&v.framebuffer(&r));
        // Change one pixel inside the visible window.
        r[0x20 * ROW_BYTES + 3] = 0x40;
        let b = fnv1a(&v.framebuffer(&r));
        assert_ne!(a, b, "the hash must move when the picture does");
        // And be stable for an unchanged picture.
        assert_eq!(b, fnv1a(&v.framebuffer(&r)));
    }

    #[test]
    fn the_frame_hash_ignores_ram_outside_the_visible_window() {
        let mut r = ram();
        let v = Video::new();
        let a = fnv1a(&v.framebuffer(&r));
        r[0] = 0xFF; // row 0 is never displayed
        assert_eq!(a, fnv1a(&v.framebuffer(&r)));
    }

    #[test]
    fn fnv1a_matches_the_published_vectors() {
        // Guards the constants: these are the canonical FNV-1a 64 test values.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    }
}
