# Crystal Castles hardware notes

Working notes toward the Phase 2 deliverable. Each entry cites both sides where
possible: the game source (`crystal-castles/`, root tree unless stated) and the
MiSTer core RTL (`Arcade-CrystalCastles_MiSTer/rtl/`). Where the two agree
independently, the fact is marked **corroborated**. Started during Phase 0; not
yet a complete hardware model description.

---

## 1. CPU memory map (program view)

From the source's own directives and guards (see `smc-gate.md` §1) plus the RTL
address decoders (`AddresDecoders.v`):

| Range | Contents | Notes |
|---|---|---|
| `0000`–`0001` | X/Y bitmap coordinate registers | `XCOORDn`/`YCOORDn`, write; `0002` = `BITMDn` bitmap data window |
| `0004`–`00EF` | zero-page game variables | |
| `00F0`–`00FF` | POKEY driver zero page | |
| `0202`–`0AFF` | lower RAM | |
| `0B00`– | POKEY voice variables | |
| `8000`–`8FFF` | additional RAM (`NRn` region, `BA[15:13]=100`, SRAM when `BA[12]=0`) | player areas, city RAM, elevators |
| `9000`–`93FF` | NVRAM/EAROM window (`NVRAMn`) | `EEROM=9000`, 256×8 |
| `9400`–`97FF` | inputs (`IN0n`) | trackball `HW.TBH/HW.TBV`, switches |
| `9800`–`9BFF` | POKEY ×2 (`CIOn`, split by BA9) | |
| `9C00`–`9FFF` | control region, decoded in 128-byte strips by `BA[9:7]` | see §2 |
| `A000`–`DFFF` | **banked ROM window, 16K** | see §3 |
| `E000`–`FFFF` | fixed ROM | vectors; `VC.IN=0E000` |

`9C00`–`9FFF` strip decode (`AddresDecoders.v:63-70`, write-strobed):

| `BA[9:7]` | Address | Strobe | Source symbol |
|---|---|---|---|
| 000 | `9C00` | UART | `EE.REC=9C00` (recall/write) |
| 001 | `9C80` | HSLD (horizontal scroll load) | |
| 010 | `9D00` | VSLD (vertical scroll load) | |
| 011 | `9D80` | INTACK | `HW.IAC` |
| 100 | `9E00` | WDOG | `HW.WDC=9E00` "woof" |
| 101 | `9E80` | OUT0 latch | §2 |
| 110 | `9F00` | OUT1 latch | |
| 111 | `9F80` | CRAM (colour RAM) | |

## 2. OUT0 addressable latch — `9E80`–`9E87` (**corroborated**)

ic8N (`CoinCountOutput.v`): 8-bit addressable latch, one bit per address,
data taken from **D0** of the written value. Write-only. Reset clears all bits.

| Address | Bit | Function (RTL) | Game source symbol |
|---|---|---|---|
| `9E80` | q0 | LIGHTBULB | `HW.TL` — trackball light, 0=off, FF=on |
| `9E81` | q1 | STARTLED2 | |
| `9E82` | q2 | EAROM store control | `EE.SLO` — store low |
| `9E83` | q3 | EAROM store control | `EE.SHI` — store high; `STORE = ~q2 & q3` |
| `9E84` | q4 | `RECALLn` | `EE.RE2` — recall, version 2 |
| `9E85` | q5 | coin counter R | `$CNCTR=9E85` |
| `9E86` | q6 | coin counter L | `$CNCTR+1` |
| `9E87` | q7 | **ROM bank select** | `HW.BSL` — "bank select 0,FF" |

The `0`/`0FF` idiom in the source works because only D0 is latched.

### 2.1 OUT1 addressable latch — `9F00`–`9F07` (**corroborated**)

ic6P (`AutoIncOutput.v`): same shape — 8-bit addressable, one bit per address,
write-only, reset clears — but it samples **D3, not D0**. `CCastles.v:332` wires
it `.BD3(BD[3])` where OUT0 at `CCastles.v:345` gets `.BD0(BD[0])`.

The difference is invisible in this game, because the source only ever writes
`0` or `0FF`, where D0 and D3 agree. It is *not* invisible to a differential
harness, and it was implemented wrongly here at first for exactly that reason —
the `0`/`0FF` idiom hides which bit is live.

| Address | Bit | Function (RTL) | Game source symbol |
|---|---|---|---|
| `9F00` | q0 | `AXn` | `HW.AX` — autoinc x, 0 on, FF off |
| `9F01` | q1 | `AYn` | `HW.AY` — autoinc y |
| `9F02` | q2 | `XINCn` | `HW.XIN` — 0 inc, FF dec |
| `9F03` | q3 | `YINCn` | `HW.YIN` |
| `9F04` | q4 | `PLAYER2` (cocktail flip) | `HW.FLP` — flip |
| `9F05` | q5 | `SIREn` | `HW.SIR` — EAROM write inhibit |
| `9F06` | q6 | `STARTLED1` | `HW.BHR` — "bothram" (**names disagree**) |
| `9F07` | q7 | `BUF1BUF2n` | `MT.BSL` — motion-object buffer select |

The `9F06` row is the one discrepancy: the RTL calls it a start LED, the game
source calls it "bothram". **UNVERIFIED** which is right; nothing implemented so
far depends on it.

## 3. ROM banking (**corroborated**)

Five 2764s (8K each). `ProgramMemory.v:126-129`:

| Window | Bank 0 (`q7=0`, reset) | Bank 1 (`q7=1`) |
|---|---|---|
| `A000`–`BFFF` (ROM0) | 1K = `136022-303` | 1H = `136022-102` |
| `C000`–`DFFF` (ROM1) | 1L = `136022-304` | 1F = `136022-101` |
| `E000`–`FFFF` (ROM2) | 1N = `136022-305` (not banked) | — |

- **Bank 0 = program** (`CRF.MAC` assembly, `.= WV.STR` = `A000`).
- **Bank 1 = castle data** (`C99.MAC` assembly: 16 playfields × `0x400` =
  exactly 16K at `.= n*WV.SIZ+WV.STR`).
- Part numbering corroborates: data ROMs kept `-1xx` numbers across revisions;
  program ROMs are `-3xx` (rev 3) in the MiSTer chip set.
- Reset state is bank 0, so the CPU boots into program.

Consumers of `HW.BSL` in the game (all ten writes):
- `CDB.MAC:7/72` — castle parser: select bank 1, read city data, select bank 0.
- `CST.MAC:314-321` — self-test: checksums each bank in turn.
- `CIN.MAC:7/39/80` — interrupt handler, see §4.

## 4. The interrupt bank-sniff (`CIN.MAC:35-41`)

The latch is write-only, and an IRQ can land while `CDB` has bank 1 mapped. The
ISR (entered via fixed ROM at `E000+`) needs bank 0 for the subroutines it calls
(`VBHAND`, `MN.SNM`, `MOOLAH` all live below `E000`), so it must discover and
restore the live bank:

```
	LDA WV.STR      ;  read A000
	IFEQ            ;  detect that using bank 1
	 TRAI 0FF IN.BSL
	 STX HW.BSL     ;  X=0: force bank 0
	ELSE
	 STX IN.BSL
	ENDIF
	...
	LDA IN.BSL      ;  restore bank select
	STA HW.BSL
	STA HW.IAC      ;  interrupt acknowledge
	STA HW.WDC      ;  woof
```

It reads `A000` and tests for zero. This works because of a **content invariant
spanning both assemblies**: the castle data bank's first byte is `0` (`C00.DAT`
opens `.BYTE 0, 0, ...`; verified) and the program bank's byte at `A000` is
nonzero. `IN.BSL` (`CG.MAC:174`, zero page) is the software shadow of the
write-only latch.

Consequences for the recompiler:
- Reads of `A000`–`DFFF` must be bank-sensitive; getting `A000` wrong silently
  corrupts the ISR's bank restore.
- The invariant is load-bearing: nothing may be relocated or reordered in a way
  that puts `0` at `A000` of the program image.
- Interrupts must be deliverable mid-`CDB`-copy (cycle accounting), or the
  bank-1 path of the sniff is dead code that suddenly matters when timing shifts.

## 5. Video

Implemented in `crates/chill65-runtime/src/video.rs`. Everything below carries a
`file:line` citation into `Arcade-CrystalCastles_MiSTer/rtl/` or is marked
**UNVERIFIED**. The distinction matters: Phase 3's differential harness compares
against this document, so a guess recorded as fact here would be self-confirming.

### 5.1 The coordinate window (**corroborated**)

Three zero-page addresses are hardware, not RAM (`AddresDecoders.v:72-74`):

| Address | Signal | Access |
|---|---|---|
| `0000` | `XCOORDn` | latches X **on write only** (`& ~BRWn`) |
| `0001` | `YCOORDn` | latches Y **on write only** |
| `0002` | `BITMDn` | bitmap data window, **read *and* write** (no `BRWn` term) |

Two consequences that are easy to get wrong:

- Reading `0000`/`0001` is an ordinary RAM read, and *writing* them also writes
  the RAM byte — the ordinary-DRAM term of `WE` (`DynamicRam.v:42`) is satisfied
  because these are not window accesses. Both effects happen.
- Any access to `0002`, **including a read**, is a bitmap access and steps the
  coordinates. A debugger must not use the normal read path.

### 5.2 Address formation and packing (**corroborated**)

`AutoIncrement.v:16`: `DRBA = ~BITMDn ? {yCoord,xCoord[7:1]} : BA[14:0]`

So the byte is at `yCoord * 128 + xCoord / 2`. 128 bytes per row, 2 pixels per
byte at 4bpp, 256 across, 256 rows — **exactly 32768 bytes**. The bitmap *is*
`0000-7FFF` in its entirety; the CPU's zero page and stack are the top-left of
the picture. Confirms the plan's 256×232-at-4bpp estimate, with 24 rows above
the visible area rather than spare capacity.

`PIXA = xCoord[0]` (`AutoIncrement.v:18`). **Even x is the low nibble, odd x the
high nibble**, and the pixel is always presented to the CPU in the *high* four
bits (`DynamicRam.v:63`), with the low four zeroed on read and **discarded on
write** (`DynamicRam.v:40`). Writes are a hardware read-modify-write that
preserves the neighbouring pixel.

### 5.3 Window writes below row 32 are dropped (**corroborated**)

`DynamicRam.v:42`:

```verilog
WE = ~BRWn & ce2Hd3 & ((BITMDn & ~DRAMn) | (~BITMDn & DRBA[14:12] != 3'b000));
```

A write *through the window* lands only when `DRBA >= 0x1000`, i.e.
`yCoord >= 32`. Rows 0–31 are silently unwritable this way, which protects zero
page, the stack and working RAM from the drawing routines. Direct stores to the
same addresses are unaffected, and **reads through the window are not gated at
all** — only writes.

Note the eight-row gap: rows 24–31 are *displayed* (§5.5) but not writable
through the window.

**This is a live tension, not a settled point.** The game's own equates
(`CG.MAC:44-46`) document the window as:

```
XB	= 0	;  bit mode X position 0-255
YB	= 1	;  bit mode Y position 24-255
VB	= 2	;  bit mode value (0F,1F,2F,...,FF)
```

So the source claims a Y range of **24**–255 while the RTL's `WE` term permits
window writes only from **32**. One of three things is true, and which is
**UNVERIFIED**:

1. the game never actually drives Y below 32 and the comment is loosely
   describing the visible screen;
2. the MiSTer core's gate is wrong (its author flagged this area as not fully
   characterised — open question **O4**);
3. this model's reading of `DRBA[14:12]` is wrong.

If the boot smoke test ever shows missing content in the top eight rows, this
is the first place to look.

Incidentally the same three lines corroborate §5.2 independently: `VB` values
are documented as `0F,1F,2F,...,FF` — the pixel in the **high** nibble with the
low nibble discarded, exactly as `DynamicRam.v:40` has it.

### 5.4 Auto-increment (**corroborated**)

`AutoIncrement.v:25-46`. Any access to `0002` steps the coordinates under four
OUT1 bits, all active-low, matching the game's equates at `CG.MAC:77-80`:

| OUT1 bit | Address | Signal | Meaning |
|---|---|---|---|
| 0 | `9F00` | `AXn` | 0 enables X step |
| 1 | `9F01` | `AYn` | 0 enables Y step |
| 2 | `9F02` | `XINCn` | 0 increments, 1 decrements |
| 3 | `9F03` | `YINCn` | 0 increments, 1 decrements |

Both axes may step on one access. Coordinates are 8-bit and wrap.

### 5.5 Visible geometry (**corroborated**)

`SyncChain.v:44-45`: `HBLANK0 = hcount[8]` gives 256 active columns of 320;
`VBLANK` is asserted for vcount 0–23 and clear for 24–255, giving **232 active
lines**. So 256×232, as the plan had it.

### 5.6 Scroll (**partly corroborated**)

- Horizontal: `hs` is seeded by `HSLD` (`9C80`) and steps once per visible pixel
  (`CCastles.v:157-170`). Since the address uses `hs[7:1]`, 256 pixel-steps
  cover exactly one 128-byte row, so horizontal scroll is a **rotation within
  the row**.
- Vertical: `VSLD` (`9D00`) loads `vr`; `vi` is reloaded from `vr` during vblank
  and steps once per line; `vs = vi <= 8'h18 ? 8'h18 : vi` (`CCastles.v:201`,
  commented "limit range to 0x18..0xFF"). `vr` **resets to `0x18`, not to zero**
  (`CCastles.v:180`). The clamp is load-bearing: `vi` is 8-bit, so a large
  scroll wraps past 255 and the bottom of the screen repeats row `0x18`.

**MEASURED:** the phase of both counters, by planting markers at known bitmap
addresses and reading back where the core put them (`flip_fixture.rs`).
Vertically, visible line *n* shows row `vr + n` — the model's assumption, now
confirmed. Horizontally the core's **first emitted column shows `hs + 2`**, not
`hs + 0`. The model uses `hs + px` and so does MAME (`effx = hscroll + x`), so
the core's bitmap scan sits **two columns** from where both models put it.

That gap has always been there and no test has ever failed for it: the only
comparison that puts our pixels beside the core's is a report rather than a
gate, and the fixture that *is* a gate draws no bitmap. It is not idle — §5.9
shows it doubling into a visible fault the moment the picture is flipped. Which
side is wrong is still open, and is the residue of open question **O4** (the
Potato chip), which the MiSTer core's own author flags as not characterised.

### 5.9 Cocktail flip (**measured**)

`PLAYER2` (OUT1 bit 4, `HW.FLP` at `9F04`) reverses both scan counters
(`CCastles.v:166`, `:194`) and mirrors each object's picture within itself
(§13.3). It does **nothing to any object's position**. Those are mirrored by
the game: `EN.PMV` in `CWV.MAC` complements each coordinate byte and adds a
constant — `0D9` vertically, `0FD` horizontally — and `CIN.MAC` adjusts the
horizontal scroll by three (`ORA #3`) and the vertical scroll not at all. MAME
agrees about the division of labour: `ccastles.cpp:480` draws every object with
`flipx = flipy = flip` at an unadjusted x and y.

The asymmetry in what the game adjusts is the whole story. Since it never
touches the vertical scroll, the hardware's vertical flip has to mirror the
visible band on to itself unaided: line *n* must show row `vr + 231 - n`, the
reverse of the upright `vr + n`. **The core counts down from `vr` instead**,
giving `vr - n` — 25 rows adrift, with the `0x18` floor smearing the 25 lines
that fall off the bottom of the count into a band of one repeated row.

Measured on the verilated core, in mirror constants: flipping maps a block
spanning `[t,b]` to `[C-b, C-t]`, and the bitmap and the objects have to agree
about `C` or they part company on screen.

| | vertical | horizontal |
|---|---|---|
| objects | 233 | 259 |
| bitmap, the core as it stands | **256** | 255 |
| bitmap, with `vi <= PLAYER2 ? vr + 8'd231 : vr` | **231** | 255 |

A half-turn maps the 232 visible lines on to themselves, so the vertical
constant has to be 231, and the one-line change reaches it: 23 lines of error
become 2. What is left over is not the flip's doing at all.

- **2 lines.** The object line buffer is filled on one line and displayed on the
  next (§13.5). A delay does not mirror, so under flip it counts twice.
- **4 pixels.** Twice the two-column bitmap phase in §5.6.

Both are invisible upright, where neither layer has anything to be measured
against. The flip is what doubles them into view — which is why the core's
author recorded this as a sprite-positioning fault, and why his own outstanding
list already suspected the sprite's horizontal location. Atari's constants say
the two layers mirror about the same number on a real board; in the core they
do not, and the residue names by how much.

The one-line change is written up for its author as
`tools/mister-cocktail-flip.patch`. It is **not applied** here: the vendored
core is an oracle and is never modified, so the measurement above verilates a
copy (`harness.md` §15.2).

**Still not modelled.** Our runtime stores the latch bit and stops there
(`harness.md` §4). Nothing above needs modelling to be true, but nothing above
is exercised by our runtime either.

### 5.7 Nibble polarity — a resolved apparent contradiction

The scanout path selects the opposite nibble from the CPU path:

```verilog
// DynamicRam.v:59  (video)   hs[0]=0 -> high nibble
if (~ce5) BIT <= hs[0] ? dout[3:0] : dout[7:4];
// DynamicRam.v:63  (CPU)     PIXA=xCoord[0]=0 -> low nibble
```

Taken literally these disagree, which would mean every pixel pair is displayed
swapped. They do not, because of the pipeline: `hs` increments on the `ce5`
cycle that *issues* the address (`CCastles.v:163`), while `BIT` is captured on
the following `~ce5` cycle, by which time `hs` has already advanced. The
selector therefore sees `hs+1`, inverting the polarity back into agreement with
`PIXA`. Both paths give **even x = low nibble**.

This is **derived, not measured** — it follows from the RTL but has not been
confirmed against a running image. If the picture ever comes out with pixel
pairs transposed, this paragraph is the thing that is wrong.

### 5.8 Colour RAM (**corroborated**)

Thirty-two entries of **nine** bits, written through the `9F80` strip.
`ColorMemory.v:29`: `din = {BA[5], BD}` at `addr = BA[4:0]` — **the ninth bit
comes from address bit 5, not the data bus**, and one write sets all nine. An
8-bit model silently loses a bit of every colour.

Read back as `o = {~rbg[8:6], ~rbg[2:0], ~rbg[5:3]}` (`ColorMemory.v:33`): bits
8–6 red, 5–3 **blue**, 2–0 **green** — green and blue are transposed on the way
out — and every component is active-low. The runtime stores the raw nine bits
without applying this transform.

### 5.9 Not cleared per frame

The bitmap is **not** cleared between frames, only between levels. `framebuffer()`
is therefore pure observation over live RAM; any implicit zeroing would destroy
game state rather than merely produce a wrong picture. There is a test for it.

## 6. Dialect traps collected so far (for `dialect.md` later)

- Every file NUL-padded to 512-byte VAX blocks.
- **`.DAT` castle files have no line terminators at all.** Corrected from an
  earlier note here claiming CR-only endings, which was inferred from a `tr`
  pipeline rather than checked. All 34 contain only printable text and NUL
  padding; record structure was VAX filesystem metadata and was lost when the
  files were archived to a flat stream. Their sole directive is `.BYTE`
  (4,484 occurrences, nothing else), so each `.BYTE` begins a record.
- `C99.MAC` switches to `.RADIX 10` around the `.DAT` includes — castle data is
  decimal, code is hex.
- Documentation prose wrapped in `.REPT 0 … .ENDR` (never assembled) in `CCN.MAC`.
- `LDAL`/`LDAH` byte-punning via `.=.-1` and `.ENABL M68` (see `smc-gate.md` §6).
- Trailing-dot decimal literals (`15.`), `==` global equates, `'` string/char
  syntax in `.PRINT`/`IF` macros.

## 7. Frame timing and the interrupt schedule (**corroborated**)

Implemented in `crates/chill65-runtime/src/frame.rs`.

### 7.1 The clock chain

`Clock.v:19-30` divides one 10 MHz master clock with a 3-bit counter:

```verilog
assign ce5  = count[0];      // 5 MHz    — pixel clock enable
assign ce2H = count == 7;    // 1.25 MHz — CPU clock enable
```

and `MicroProcessor.v:30` wires `.RDY(ce2H)`. **The CPU therefore runs at
1.25 MHz — one CPU cycle per four pixels.** That divisor is the number
everything else hangs off, and it is stated by the RTL rather than inferred.

### 7.2 The derivation

`SyncChain.v:30-33` steps `hcount` on `ce5`, wrapping at 320 and stepping the
8-bit `vcount`:

```text
  320 * 256          = 81,920 pixel clocks per frame
  5,000,000 / 81,920 = 61.035 Hz      (core README: "61.03 Hz")
  320 / 4            = 80 CPU cycles per line
  80 * 256           = 20,480 CPU cycles per frame
  1,250,000 / 20,480 = 61.035 Hz      — closes exactly, both ways round
```

Note 256 lines, not 232: `vcount` is 8 bits and the frame is the full count.
232 is only the *visible* subset (§5.5).

### 7.3 IRQ — four per frame, latched

`SyncChain.v:45`: `assign IRQCLK = ~vcount[5];  // every 64 lines (4x per frame)`

`~vcount[5]` is high for lines 0–31, 64–95, 128–159, 192–223, so it **rises
entering lines 0, 64, 128, 192** — four evenly spaced interrupts per frame, at
CPU cycles 0, 5120, 10240 and 15360 from frame start.

`MicroProcessor.v:38-46` latches on that rising edge and clears only on INTACK:

```verilog
else if (~INTACKn)  IRQ <= 1'b0;
else begin IRQCLK2 <= IRQCLK; if (~IRQCLK2 & IRQCLK) IRQ <= 1'b1; end
```

Two consequences, both modelled and both tested:

- **IRQ is a level, not a pulse.** Raising it while `I` is set does not lose it;
  the CPU takes it the moment `I` clears. Only the `9D80` strobe clears it.
- **A handler that does not acknowledge is re-entered** as soon as `RTI`
  restores `I` — continuously. This is hardware behaviour, not a modelling
  artefact, and it is why `CIN.MAC:82-83` strobes INTACK and the watchdog
  before restoring registers.

### 7.4 NMI is unreachable

`MicroProcessor.v:26` ties the line low: `.NMI(1'b0)`. The vector at `FFFA`
exists and points at `E009`; nothing can ever reach it. The scheduler does not
raise NMI and must not be "fixed" to do so — there is a test guarding this.

### 7.5 Instruction granularity

Interrupts are checked at instruction boundaries, as on real hardware, so a
line deadline can fall mid-instruction and a frame may overshoot 20,480 by up
to one instruction (≤6 cycles, under 0.03%). Deadlines are absolute within the
frame rather than accumulated per line, so the overshoot does not compound
across the 256 lines; each IRQ still lands within one instruction of its true
cycle. Tested over 20 consecutive frames.

## 8. POKEY ×2 (**verified against the core**)

Implemented in `crates/chill65-runtime/src/pokey.rs`.

Audio *was* deliberately absent: plan §5 calls POKEY "not the interesting part"
and the Phase 2 gate is headless, so there was no output device to feed. That is
no longer true — §8.7 onward describe the synthesis and how it was checked. The
register-level facts in §8.2–8.6 are unchanged and still hold.

### 8.1 `RANDOM` is gameplay, not noise

`RANDOM` was implemented properly even while audio was not, because the game
reads it to make **gameplay** decisions rather than merely noise:

- `CATOUT.MAC:22` — `LDA RANDOM / AND #0E0 / ADC #010 / STA XB` chooses the
  cat's X position on the bitmap. A stubbed constant would spawn every cat in
  the same column, which would look like a game bug, not a missing peripheral.
- `CCUBE.MAC` reads it seven times.

### 8.2 Addressing

`AddresDecoders.v:59` places both chips in `9800-9BFF`; `AudioOutput.v:16-17`
splits them on `BA[9]`:

| Window | Chip | Game symbol |
|---|---|---|
| `9800`–`99FF` | 3B | `POKEY0 = 9800`, `RANDOM = 0A+POKEY0`, `POKCT0 = 0F+POKEY0` |
| `9A00`–`9BFF` | 3D | `POKEY1 = 9A00`, `POKCT1 = 0F+POKEY1` |

Each chip receives only `BA[3:0]` (`AudioOutput.v:28`), so its sixteen registers
**mirror every 16 bytes** across its 512-byte window. Game equates at
`CG.MAC:34-40` agree.

### 8.3 The poly advances once per CPU cycle

`AudioOutput.v:26` gives the chips `.ce(ce2Hd)` — the delayed 1.25 MHz CPU
enable — and `SupportChips.v:240` ties `.enable_179(1'b1)`. There is no further
division: **one poly step per CPU cycle**. The model advances it lazily on
access, which is exactly equivalent because the state is a pure function of the
elapsed cycle count.

### 8.4 The LFSR

Transcribed literally from `Pokey/pokey_poly_17_9.v:47-70` rather than from
prose, since the published descriptions of POKEY's poly17 disagree with one
another. Points worth stating:

> **A sourcing exception.** That file is one of Mark Watson's, which §8.7 says
> cannot be a source here. This transcription predates that rule and is left in
> place rather than quietly rewritten, because it is a decision to take rather
> than a bug to fix. It can be settled without changing a line of it: distortion
> `AUDC = 0x8` puts poly17 on the output the same way `0xC` puts poly4 there, so
> the sequence can be measured off the core and this implementation either
> confirmed or replaced. Noted here so the inconsistency is visible rather than
> buried.

- Reset word is `17'b01010101010101010` = `0xAAAA` (`:39`).
- Feedback is `shift_reg[13] ~^ shift_reg[8]` — **XNOR, not XOR** (`:47`). From
  the reset word bit 13 is set and bit 8 clear, so the two differ immediately;
  there is a test pinned on exactly that.
- `RANDOM` is `~shift_reg[15:8]` (`:73`) — **inverted**.
- Measured periods: **131,071** (2¹⁷−1) in 17-bit mode, **511** (2⁹−1) with
  `AUDCTL` bit 7 set. Measured in tests rather than asserted from memory.
- The 9-bit period only appears after ≥8 steps of settling: bits `[16:8]` are
  the self-contained 9-bit LFSR, but `[7:0]` are a *delayed copy of the feedback
  stream* and take eight steps to become consistent with it. Sampled earlier,
  the full 17-bit word never returns to its start and the period looks
  unbounded.

### 8.5 Init mode is the state at power-on

`pokey.v:709`: `initmode = ~(skctl_next[1] | skctl_next[0])`. While both low
bits of SKCTL are clear the poly is held with bit 16 forced low.

This is not a corner case. From reset SKCTL is zero, and the game zeroes all
sixteen registers of both chips (`CST.MAC:10-15`) before writing **`SKCTL = 7`**
("FAST POT", `CST.MAC:18-19`) to release them. So the poly is genuinely held
until the self-test runs, and any determinism argument has to account for it.

### 8.6 Idle reads

POT0–7 and ALLPOT read zero — this board reads a trackball, not paddles, and
zero from ALLPOT means "no pot still counting", so anything polling it for
completion proceeds rather than hangs. IRQST reads `0xFF` (active-low: nothing
pending); POKEY's IRQ is not wired to this CPU in any case, since interrupts
come from the video counters (§7.3). SKSTAT reads `0xFF`.


### 8.7 Audio: measured, not transcribed

Everything from here down was recovered by **running original fixtures on the
verilated core and reading its `SOUT` output back**, not by reading its POKEY
sources. That is a licence constraint with teeth: `rtl/Pokey/` is © 2013 Mark
Watson, licensed for non-commercial use only with the notice extending to
derived works, and this repository is MIT. Observing what hardware does is a
different act from copying how someone expressed it, and only the first is
available here.

The practical consequence is that every claim below is backed by a measurement
that can be repeated, and the ones that could **not** be measured say so rather
than being filled in from a datasheet. Where measurement and the published
descriptions of the chip agree, that agreement is itself evidence and is noted.

### 8.8 The free-running parts

**Two clock divisors**, both measured. A pure tone with `AUDF = 0` toggles once
per divider tick, so its period is two ticks:

| `AUDCTL` bit 0 | measured period | divisor |
|---|---|---|
| clear | 56 cycles | **28** |
| set | 228 cycles | **114** |

Their ratio, 4.07, is the documented 63.9 kHz : 15.7 kHz relationship — the
cross-check that these are POKEY's two audio clocks and not some other pair.
Note that this board clocks POKEY at the 1.25 MHz CPU rate rather than 1.79 MHz
(§8.3), so the *divisors* are the chip's and the resulting frequencies are not:
44.6 kHz and 11.0 kHz.

**Two polynomial counters**, `x⁴+x³+1` (taps 0,1, XOR) and `x⁵+x³+1` (taps 0,2,
XNOR) — the standard maximal polynomials, which is independent agreement between
measurement and the published descriptions.

Recovering them took three steps, because neither can be read off directly:

1. **They free-run.** The channel samples the polynomial when its divider
   underflows, so what reaches the output is a *decimation*. Changing `AUDF`
   changes the sequence, which is what a free-running register does and a
   divider-stepped one does not.
2. **They run at the CPU clock, not the base clock.** With a channel on the main
   clock (`AUDCTL` bit 6) the output period is 60 cycles at `AUDF = 0` and 120
   at `AUDF = 4` — fifteen underflows either way. Fifteen distinct values inside
   60 cycles is impossible for a register stepping once per 28-cycle base tick.
3. **Two decimations solve them.** The slow path samples every 28 cycles and the
   fast path every 4; both are invertible modulo 15 and 31, so each reading
   un-decimates into a candidate register. They agree up to rotation — two
   unrelated sampling rates cannot produce consistent nonsense.

poly5 needed one extra step, because it never reaches the output at all: it
*gates* the stage behind it, so its bits are in the output's **transitions**.
Its polarity has a second, independent witness — XNOR excludes the all-ones
state where XOR excludes all-zeros, giving 15 ones per 31 terms rather than 16,
and an odd count per period is exactly what makes the gated output's period 62
ticks instead of 31.

**No delayed taps are modelled.** Plain undelayed registers reproduce both
captured sequences term for term. A delay in the real part would appear as a
constant phase offset, which is what §8.12's alignment measures — so if one
exists it shows up in a place built to measure it rather than being guessed at.

### 8.9 The per-channel pipeline

Per chip: four channels, each an 8-bit divider feeding an output flip-flop
feeding a volume gate, summed to the chip's six-bit `snd`.

**Dividers**, measured: a channel underflows every `AUDF + 1` base-clock ticks,
or every `AUDF + 4` CPU cycles on the main clock.

**Distortion**, from `AUDC` bits 7:5 — the whole table measured, by sweeping all
eight settings and recording each period in divider ticks:

| `AUDC` | period | source |
|---|---|---|
| `0x0` | none | poly5 + poly17 |
| `0x2` | 62 | poly5 + pure tone |
| `0x4` | 465 = 15×31 | poly5 + poly4 |
| `0x6` | 62 | *identical to* `0x2` |
| `0x8` | none | poly17 |
| `0xA` | 2 | pure tone |
| `0xC` | 15 | poly4 |
| `0xE` | 2 | *identical to* `0xA` |

The two collapsed pairs are the finding: **bit 7 selects whether poly5 is in the
chain**, and bits 6:5 pick poly17, poly4 or a pure tone after it. With poly5 in
circuit the flip-flop holds rather than updates when its bit is clear, which is
why those settings measure twice the period. The pure-tone case toggles; the
polynomial cases take the polynomial's bit, which is why poly4 alone measures 15
rather than 30.

**High-pass**: `AUDCTL` bit 2 filters channel 1 against channel 3 and bit 1
channel 2 against channel 4 — a flip-flop samples the filtered channel at each
of the clocking channel's underflows, and the two are XORed. **Not measured**;
the game is not known to use it.

**16-bit joins**: `AUDCTL` bit 4 joins channels 1+2 and bit 3 joins 3+4 into one
divider clocked by the low channel. **Not measured** for the same reason.

**Volume**: `AUDC` bits 3:0 are the level and bit 4 is volume-only, emitting the
level continuously and ignoring the flip-flop — which is how software plays
sampled sound.

**Summation**: four channels of 0–15 make the chip's six-bit `snd`, 0–60, and
the board sums both chips as `SOUT = {2'b00,snd1}+{2'b00,snd2}`
(`AudioOutput.v:51`), 0–120 in eight bits. That last is a wiring fact about this
board, readable from a file that is not one of the restricted ones.

### 8.10 `STIMER` does two things, and both were found the hard way

Writing `STIMER` (register `09`, write-only, sharing an address with `KBCODE`'s
read) restarts the channels so software can stop them beating against each
other. Two details were wrong until the whole path was compared at once, and
neither could have been caught by measuring one signal in isolation:

- **It restarts the base-clock prescaler**, not only the channel counters. The
  fixture strobes one chip and then the other, four cycles apart — one `STA` —
  and the core puts the two chips' first edges exactly those four cycles apart.
  A free-running prescaler would have both chips share the 28-cycle grid and
  fire together however they were strobed.
- **It sets the output flip-flops rather than clearing them.** With them
  cleared, every tone came out inverted against the core: identical periods,
  identical run lengths, opposite phase by exactly half a period each. It cannot
  instead be a global output inversion, because poly4's sequence was identified
  from the core's own output and inverting would break it.

### 8.11 What is not modelled, and why it cannot matter

The serial port, two-tone mode, the keyboard scan and the pot counters are
absent. This board's driver writes `SKCTL = 7` (`CST.MAC:18-19`) and never
enables any of them — bits 6:4 and 3 stay zero for the life of the program — and
nothing reads back a serial or two-tone result, so there is no path by which
their absence produces a silently wrong answer. POKEY's IRQ is not wired to this
CPU at all; interrupts come from the video counters (§7.3).

Three things *within* the audio path are honestly unestablished, and each says
so where it is implemented rather than only here:

- **poly17's audio tap** — which bit of that register the audio path uses. §8.4
  models poly17 for `RANDOM`, which reads a different field entirely. This is
  the one input to the pipeline that is a guess.
- **The joined 16-bit modes** and **the high-pass filters**, neither exercised
  by any fixture.

### 8.12 How it was verified

`crates/chill65-diff/fixture/audio.MAC` is an original program of ours — no game
bytes, no corpus needed. It programs both chips (two pure tones at different
frequencies so they beat, plus a volume-only DC level), strobes `STIMER` and
idles. Both implementations run it and their per-cycle `SOUT` streams are
compared byte for byte:

```
audio fixture: aligned at lag 665, 40000 consecutive samples identical
control (AUDF0 5 -> 6): aligned at None
```

The two sides cannot share a cycle origin — ours starts at the CPU's first cycle
after reset, the core's recording at the tick after `reset_n` is released — so
the test searches for the constant lag that aligns them, asserts it is small and
bounded, prints it, and then requires **exact** equality across the window. A
tolerance nobody derived would be a weakened gate; there is none.

The control detunes one `AUDF` byte in our own program and requires the
comparison to stop aligning at any lag. It does. A gate that cannot fail is not
a gate.

The fixture deliberately selects **no polynomial distortion**. The polynomials
are measured and tested, but their *phase at reset* is not established — the
fixtures that identified them idled for tens of thousands of cycles first, so
they read the sequence and not its origin. Including one would compare our
arbitrary starting phase against the core's real one and fail for a reason
outside the audio path.

### 8.13 Where the sound comes out

The runtime **produces samples and plays nothing**, the same seam the EAROM and
the clock sit on: it must build for `wasm32-unknown-unknown`, so it has no audio
device, no filesystem and no clock. `run_frame` leaves one `u8` per CPU cycle in
a buffer the embedder reads — 20,480 per frame plus the overshoot §7.5
documents, since interrupts are checked at instruction boundaries.

It is **off by default** and observationally free when off. With it on, frame
hashes and cycle counts are bit-identical to a run with it off, and there is a
test asserting exactly that: audio changes no picture.

The browser page resamples the 1.25 MHz stream to the audio device's rate and
schedules it against the `AudioContext` clock, which is not the clock game
frames are driven from. `wasm.md` has that end of it.

## 9. Inputs — switches and trackball (**corroborated**)

Implemented in `crates/chill65-runtime/src/input.rs`.

### 9.1 Address map

`IN0n` covers `9400-97FF`; `CCastles.v:114-117` splits it on `BA[9]`:

| Window | Contents |
|---|---|
| `9400`–`95FF` | LETA trackball interface, `addr = BA[1:0]` (`CCastles.v:294`) |
| `9600`–`97FF` | player switches |

Only two of the LETA's four axes are wired (`CCastles.v:292-293`): axis 1 is
vertical, axis 2 horizontal, axes 3 and 4 tied off. So register 0 is vertical
and register 1 horizontal, agreeing with `HW.TBV = 9400` / `HW.TBH = 9401`.

**Mind the axis order.** `CG.MAC` holds *two* equate blocks under conditional
assembly and they **swap the axes**: the wirewrap block at `:62-63` has
`HW.TBH = 9400`, the production block at `:120-121` has `HW.TBV = 9400`.
`CG.MAC:5` sets `CG.PC = 1` — "PC = 0 wirewrap, 1 PC board" — so `.IF EQ,CG.PC`
is false and **the production block is live**. Taking the wrong one puts
vertical motion on the horizontal axis, which would look like a control bug.

### 9.2 The switch byte at `9600`

`CCastles.v:102`:

```verilog
wire [7:0] playerSwitches =
    { ~STARTJMP2, ~STARTJMP1, VBLANK, ~SELFTEST, ~SLAM, ~COINA, ~COINL, ~COINR};
```

| Bit | Signal | Game symbol |
|---|---|---|
| 7 | `~STARTJMP2` | `MA.ST2`/`MA.BU2` — right start *and* jump |
| 6 | `~STARTJMP1` | `MA.ST1`/`MA.BUT` — left start *and* jump |
| 5 | `VBLANK` — **not inverted** | `MA.VBL = MD5` (`CG.MAC:124`) |
| 4 | `~SELFTEST` | `MA.STS = MD4` (`CG.MAC:134`) |
| 3 | `~SLAM` | |
| 2 | `~COINA` | |
| 1 | `~COINL` | |
| 0 | `~COINR` | |

Every bit is active-low **except bit 5**, which is a live video signal rather
than a switch, and which the frame scheduler drives from the scanline (§7.2).

Three-way corroboration: the RTL, the game's `MA.*` masks, and the game's coin
configuration flags — `COIN01 = 1 ; COINS ARE IN D0-D2` and
`COIN = 0 ; COIN SWITCHES ARE ACTIVE LOW` (`CG.MAC:334-336`).

Start and jump are the **same physical input**; the RTL name `STARTJMP` says so,
and the game maps `MA.ST1` and `MA.BUT` to the same bit `MD6`.

### 9.3 The counter wraps; the *game* saturates

`SupportChips.v:297-307` — the quadrature decoder is a free-running **9-bit**
up/down counter with no saturation and no reset, exposed as
`count = counter[8:1]`:

```verilog
reg [8:0] counter;
if(ce) begin if(dir) counter <= counter + 1; else counter <= counter - 1; end
assign count = counter[8:1];
```

It is an **absolute position**, not a delta. The game differences it itself
(`CEN.MAC:317-330`):

```text
TR.DEL:  LDA HW.TBH
         SUB TR.I     ; delta = current - previous
         STA TR.XD
         ...
         LDA HW.TBH
         STA TR.I     ; remember for next time
```

The 8-bit subtraction wraps, which is exactly why a wrapping counter is right:
the wrap is invisible to the game so long as movement between samples stays
under 128 counts. Clamping is then done **in software** by `GP.TRC`
("truncates to -TEMP1:TEMP1", `CEN.MAC:338-350`).

**So the hardware must not saturate.** A clamp here would corrupt the game's own
delta arithmetic and present as a control-feel bug rather than a modelling
error. This resolves the "wrap/saturation" question in the task that raised it:
wrap in hardware, saturate in software.

### 9.4 What `TrackballEmu.v` is, and is not

It is **not** evidence about the arcade hardware. It is the MiSTer core's
adapter converting a modern mouse or joystick into quadrature pulses, complete
with a sensitivity setting and a falloff ramp (`TrackballEmu.v:22-26`). Those
are host conveniences and are deliberately not reproduced.

### 9.5 Presentation and the absence of acceleration

Per plan §6, host movement accumulates between frames and is presented to the
CPU **once per frame**, folded in by the frame scheduler. **No pointer
acceleration or smoothing is applied anywhere** — the plan calls this out
because acceleration silently changes game feel and is very hard to diagnose
afterwards. There is a test asserting that ten one-unit moves and one ten-unit
move are indistinguishable.

**UNVERIFIED:** the scaling between one host delta unit and one CPU-visible
count. The model treats them as equal, bypassing the decoder's internal ÷2,
because a host API supplies movement rather than quadrature edges. Feel-tuning
belongs in a front end, not in the machine model.

## 10. The power-up path (**corroborated**)

Established while building the boot smoke test
(`crates/chill65-runtime/tests/boot.rs`), and load-bearing for §11's self-test
work.

### 10.1 Reset always runs the self-test

`CIN.MAC:5-8` — the reset vector lands at `E000`, which must be in the fixed
ROM because it switches the bank under its own feet:

```text
START:      ;  this must be at E000 so no bank select
            ;  problems at power up
    TRAI 0 HW.BSL   ;  select bank 0 and jump
    JMP MN.PWR
```

`MN.PWR` (`CIN.MAC:183`) initialises the EAROM, waits for vblank, sets the
scroll and auto-increment latches, and then ends on a **conditional-assembly**
branch (`CIN.MAC:222-227`):

```text
    .IF NE,CG.ST
    .IFT
    JMP MN.ST       ; power-on self-test
    .IFF
    JMP MN.HIC      ; straight to the game
    .ENDC
```

`CG.MAC:9` sets **`CG.ST = 1`** in both the root tree and `version-3`, so the
shipped ROM **always runs the power-on self-test first**. This is not the
operator self-test switch: that is a separate runtime check at `CMN.MAC:772-776`
(`LDA HW.STS / AND #MA.STS / IFEQ / JMP MN.SLT`), which enters self-test when
bit 4 reads **zero** — and bit 4 is `~SELFTEST`, so an open switch reads 1 and
the game proceeds normally.

### 10.2 Observed boot timeline

Measured on this model, at 61.035 Hz:

| Frame | Event |
|---|---|
| 0 | reset at `E000`, one write to `HW.BSL` (bank 0, already live) |
| ~0–160 | self-test: POKEY test, then `CST.MAC:20`'s "WAIT FOR 80 VBLANKS" (127) |
| **162** | first change to the framebuffer |
| **353** | first interrupt taken — the game has reached `MN.HIC`'s `CLI` (`CMN.MAC:32`) |
| 420 | 200 interrupts, 12 real bank switches, 28,681 lit pixels |

**Interrupts stay masked for the first ~5.8 seconds.** Anything that samples the
machine earlier than frame 353 will see no interrupts and a mostly blank screen,
which is *correct* but indistinguishable from a wedged machine — the boot test
therefore runs 420 frames and asserts on drawing and interrupts, not merely on
the absence of a crash.

The 127-vblank wait polls `HW.VBL AND #MA.VBL`, so it also exercises §9.2's
VBLANK bit with real game code: if that bit did not toggle, the machine would
spin there until the watchdog expired.

## 11. The self-test ROM check — the objective gate (**corroborated**)

`crates/chill65-runtime/tests/selftest.rs`. This is the strongest correctness
evidence in the project: a diagnostic written by Atari, run on our machine,
with a binary verdict. "Playable" cannot be asserted in a test; this can.

### 11.1 Entry: no switch is involved

Self-test is **not** entered by an input. Per §10.1, `CG.MAC:9` sets `CG.ST = 1`
and `CIN.MAC:222` therefore compiles `JMP MN.ST` into the power-up path
unconditionally. The `HW.STS` switch at `CMN.MAC:772` is a separate runtime
check for re-entering self-test during play.

### 11.2 What it computes

`CST.MAC:312-323` — `ROMTST` scans both banks, 32 pages per 2764:

| Pass | Bank | Pages | Range |
|---|---|---|---|
| 1 | 0 (`STA HW.BSL` with 0) | `20*NPROM1-1` = 95 → 3 ROMs | `A000-FFFF`, 24K |
| 2 | 1 (`TRAI 0FF HW.BSL`) | `20*NPROM2-1` = 63 → 2 ROMs | `A000-DFFF`, 16K |

`ROMTS1` (`CST.MAC:324-350`) accumulates a longitudinal parity with `EOR NY,TEMP1`
across each 8K device, reseeding with `0FF` at each 32-page boundary and storing
into `TEMP3(Y)`. It strobes `HW.WDC` once per page, which is what keeps the
watchdog quiet through a 40K scan (§7 and the watchdog constant depend on this).

### 11.3 The observable

`CST.MAC:352-369` — `GOTCHK` requires **`TEMP3[i] - 1 == i`** for `i` in 0..4,
so the five checksums must be exactly **`1, 2, 3, 4, 5`**. That the expected
value is the index plus one is not a coincidence: it makes the displayed
checksum identify *which* device failed.

- **Pass** → `LDA #29` ("ROM OK"), `JSR MS.DRW`, `JMP MOREST` (`EDC6`).
- **Fail** → `10$` (`ED37`), draws the checksums, then `17$: BCS 17$` (`ED7F`) —
  an infinite spin that deliberately lets the watchdog bite.

Symbol addresses, from the assembler's own `--symbols` output: `ROMTST=ECC9`,
`ROMTS1=ECE7`, `GOTCHK=ED17`, `MOREST=EDC6`, `TEMP3=00AE` (a five-byte array
that overruns into `TEMP4`/`TEMP5`).

### 11.4 Result

```text
self-test: ROMTST at 6,451,839 cycles, GOTCHK at 6,866,406
           (414,567 cycles scanning 40K across both banks)
           verdict Passed { checksums: [1, 2, 3, 4, 5] }
           1062 watchdog strobes, 4 bank writes (2 switches)
```

**The game's own ROM test passes on all five devices**, having switched to bank 1
and back. Because the scan spans both banks, this exercises bank-sensitive reads
end to end — the failure mode §4 warns about (a bus that ignores the bank) would
corrupt 16K of the scan and be caught here.

### 11.5 The gate has teeth

A gate that cannot fail proves nothing, so there is a negative control:
corrupting a single byte at offset `0x0100` of the program image yields

```text
corrupted ROM correctly rejected: checksums [00, 02, 03, 04, 05]
```

The game not only rejects it but **names the right device** — offset `0x0100` is
in `A000-BFFF`, the first 2764, and index 0 is the one that mismatches.

## 12. Colour decode and the headless runner

`crates/chill65-runtime/src/bin/ccrun.rs` runs the machine headless and can dump
the visible picture as a binary PPM — the human-inspectable artefact for the
later, human-judged "playable" milestone. No window, no audio, no third-party
crate (PPM is used precisely because it is trivial to write by hand).

```text
ccrun <prog.bin> <data.bin> [--frames N] [--dump out.ppm] [--quiet]
```

### 12.1 Which CRAM entry a bitmap pixel selects (**derived**)

`ColorMemory.v:22-31` arbitrates the colour address between the bitmap and the
motion objects. With `MV = 3'b111` — the "no object here" encoding, since every
term of `sel` contains some `~MV[i]` — `sel` collapses to 0, `A4` is driven high
by its last term `MV[2] & MV[1] & MV[0]`, and `A3:A0` pass `BIT[3:0]` through
unchanged. The address is therefore `{1, pixel}`: **entries 16–31**.

This is **derived, not measured**. It is also now a special case of something
implemented in full: see §13.

### 12.2 Decoding an entry (**corroborated**)

`ColorMemory.v:33`: `o = { ~rbg[8:6], ~rbg[2:0], ~rbg[5:3] }`. Three traps, all
of which a naive reading of "rbg" would fall into:

- every component is **active-low**;
- **green** comes from the bottom three bits;
- **blue** comes from the middle three — green and blue are transposed.

Getting any of these wrong yields a garish picture rather than a subtly wrong
one, which is a mercy. The rendered attract screen shows grey castle walls, red
turret tops, tan paths and white text, which is Crystal Castles.

**How three bits become eight is not in the RTL** (**measured**). `ColorMemory.v`
emits three bits per channel; the expansion is the resistor ladder on the way
out, and this model used a linear `v * 255 / 7` until it was compared against
MAME, which models the ladder. Measured correspondence:

```text
3-bit   0     3     4     6     7
ours    0   109   145   218   255      (was: linear)
MAME    0   104   151   223   255
```

Those five fit per-bit contributions of **151, 72 and 32**, summing to exactly
255, whose conductance ratios 4.72 : 2.25 : 1 are a **1 kΩ / 2.2 kΩ / 4.7 kΩ**
ladder — the ordinary Atari arrangement, so the fit is a resistor network rather
than a curve. `video::DAC` carries the table. **1, 2 and 5 are derived from
those weights, not measured**: the game's palette selects them in no committed
trace, so MAME was never asked.

The error was five counts at worst and invisible to every lit-pixel measurement,
which counts non-black. It was **not** invisible to pixel equality: it made
roughly 20,000 of 59,392 pixels differ from MAME on a picture that was otherwise
identical, and it had done so since Phase 3.

### 12.3 Determinism (**verified**)

600 frames of attract mode produce frame hash `087e02f4ca003874` with 33,238 lit
pixels, identically across independent runs, and the same value from `ccrun` and
from the test harness.

This matters more than it looks. The game reads POKEY `RANDOM` for gameplay
decisions (§8.1), so a reproducible hash is evidence that the LFSR is clocked
from machine cycles and that nothing host-dependent has leaked in. Phase 3's
differential harness against MAME cannot be built without it.

The test also runs 300 frames and asserts the hash **differs**, so the equality
above cannot be satisfied by a machine that simply never changes.

## 13. Motion objects (**verified against the core**)

The bitmap draws the castle and the crystals. Everything that moves — Bentley
Bear and everything chasing him — is a **motion object**, drawn by separate
hardware and composited over the bitmap. Modelled in
`chill65-runtime/src/motion.rs`, and **verified**: the model agrees with the
verilated MiSTer core to **zero differing pixels** over the 252 columns the core
emits, on a fixture that plants nine objects (`chill65-diff/tests/mob_fixture.rs`).

### 13.1 The table

`WorkingRam.v:16` addresses SRAM from the video side as
`{3'b111, BUF1BUF2n, HC[8:2]}` — an 11-bit *word* address. `BUF1BUF2n` is OUT1
bit 7, `MT.BSL` at `9F07` (§4), so the table is one of two 256-byte halves:

| `MT.BSL` | SRAM | CPU |
|---|---|---|
| 0 | `0xE00–0xEFF` | `8E00–8EFF` |
| 1 | `0xF00–0xFFF` | `8F00–8FFF` |

`HC[8:2]` steps one word every four pixel clocks and `PositionControl.v` loads a
horizontal counter every eight (`HC[2:0] == 101`), so an object is **two words,
four bytes**, and exactly **40** are scanned per line — `HC` runs 0–319. Entries
40–63 exist in RAM and never draw.

`WorkingRam.v:41` gives `SR = {ic6D, ic6B}` with `ic6B` holding even addresses
and `ic6D` odd, so a word's low byte is even and its high byte odd. With which
phase reads which word, that fixes the entry:

| Byte | Meaning | Source |
|---|---|---|
| 0 | picture code | `MotionObjectPictureRom.v:16-19`, latched from `SR[7:0]` |
| 1 | vertical position | `MotionObjectVerticalControl.v:13`, read as `SR[15:8]` |
| 2 | bit 7 = `MPI` | `MotionObjectBuffer.v:14-18`, latched from `SR[7]` |
| 3 | horizontal position | `MotionObjectHorizontalControl.v`, loaded from `SR[15:8]` |

### 13.2 Vertical extent

`MotionObjectVerticalControl.v:13-20`: `sum = VC + SR[15:8]`, on this line while
`sum[7:4]` is all ones, and `q = sum[3:0]` is the row. Sixteen values of `VC`
put the sum in `F0..FF`, so an object is **sixteen lines tall**. A non-matching
object forces every plane to `1111` (`MotionObjectPictureRom.v:68-70`), which is
colour 7 — transparency is how "no object" is expressed, not a separate enable.

### 13.3 The picture ROMs

`addr = {picture, q ^ {4{PLAYER2}}, ~CK1 ^ PLAYER2}`, thirteen bits, one 2764.
Each fetch yields three bit-planes of four pixels: `nib3 = data_ic8D[3:0]`,
`nib2 = data_ic8B[7:4]`, `nib1 = data_ic8B[3:0]`. **Only the low nibble of `8D`
is wired**; its high nibble goes nowhere. Two fetches make an object **8 wide by
16 tall at 3 bits per pixel**, from `136022-106.8d` and `136022-107.8b`.

`PLAYER2` (OUT1 bit 4, `HW.FLP`) mirrors both axes: it XORs the row with `0xF`,
swaps which half is fetched first, and reads the shifters LSB-first instead of
MSB-first (`MotionObjectPictureRom.v:104`). It mirrors the picture **inside**
the object and leaves the object where it is; the game moves it (§5.9).

### 13.4 Arbitration, and what `MPI` means (**verified**)

`ColorMemory.v:16-21`, implemented exactly in `video::cram_address` and checked
against an independent re-evaluation on all 256 inputs. Three regimes:

- `MV == 7` — the bitmap wins at `BITMAP_CRAM_BASE + BIT`, whatever `MPI` is.
  This is §12.1's derivation as live code.
- `MV < 7` with `MPI` clear, or over a dim bitmap pixel — the object wins, at
  `{0, MPI, MV}`: entries 0–15.
- `MV` in 1..=6 with `MPI` set over a bright pixel — the **bitmap** wins. So
  `MPI` is a *behind-bright-bitmap* priority.

**`MV == 0` is an exception to the last rule**: the first `sel` term is
`(~MV[2] & ~MV[1] & ~MV[0])`, true only for `MV == 0`, and it forces the object
through regardless of priority, landing at address 8. That falls out of the
equations rather than being a special case anyone wrote.

### 13.5 The one-line delay, and when the table is read (**measured**)

Two line buffers alternate on `VC[0]` (`PositionControl.v`), one written while
the other is read out and blanked behind itself (`MotionObjectBuffer.v:20`). A
buffer filled while the counter reads `VC` is **displayed on line `VC + 1`** —
`motion::DISPLAY_DELAY_LINES`.

**When the table is sampled matters, and this part is measured rather than
derived.** The game builds the table, the video reads it as the field is
scanned, and the game parks it again — every entry `F0`, whose match window is
`VC 00–0F` and so reaches no visible line — before the frame ends. A model
compositing from SRAM at end of frame therefore sees an empty table. So
`Machine::latch_motion_objects` copies the table and OUT1 at
`FIRST_VISIBLE_LINE`, where the hardware starts reading it.

A consequence worth stating because it looks like a fault and is not:
**early attract mode shows no characters.** The table is parked through it —
measured on `idle-attract`, empty at frame 600 — so the castle draws with nobody
in it. The demo later in the attract cycle does have characters: at frame 1100,
about 18 seconds in, our picture gains 570 lit pixels and MAME's gains 566. Both
sides, the same frame.

So "no characters" is true of the first part of attract and false of the rest,
and the two are easy to mistake for a fault in whichever one you happen to look
at.

### 13.6 What is not modelled

The model composites **once per frame**, from the latched table. There is no
scanline-level video model, and `harness.md` §13.6 records why building one is
out of scope. A game that changed the table part-way down the field would be
drawn as though it had not.

## 14. Space Duel's vector generator — deflection full scale (**corroborated**)

Note that §§1-13 above are Crystal Castles, which has a bitmap. This section is
Space Duel, which has none: the CPU builds a display list and the generator
deflects a beam. `vg.rs` documents the instruction set; this is about how far
that beam goes.

### 14.1 The number, and where it comes from

**The visible field is 512 by 384 generator units — ±256 by ±192 about the
centre** — after the generator's scale is applied.

The primary evidence is the board's own diagnostic. `XYSIG.MAC` is titled
"COLOR XY SIGNITURE ANALASIS" and its header names the hardware: "SPACE DUEL XY
COLOR GRAPHICS BOARD". It tests the long-vector instruction by driving the beam
from the centre to a corner (`XYSIG.MAC:143-145`):

```text
TEST2:  CNTR
        SCAL 1
        VCTR 512., 384., 1
```

`SCAL 1` is binary scale 1, which halves (`VGMC.MAC:77`), so the deltas actually
drawn are 256 and 192. The trailing dots are MACRO-11 decimal overrides — the
file is in `.RADIX 16` around them. A diagnostic drawing a corner is drawing the
edge of the deflection it has; the same vector is drawn back again, negated, at
`XYSIG.MAC:147`, so the pair sweeps a full diagonal.

Our decoder agrees, which is the point of testing it:
`vg.rs`'s `the_diagnostic_corner_vector_lands_on_full_scale` runs exactly those
words and lands on (256, 192).

### 14.2 The game's own use agrees, independently

`AST2RD.MAC:575-576` sets the playfield's edges:

```text
XRIGHT  =20     ;RIGHT SIDE OF SCREEN
YTOP    =18     ;TOP OF SCREEN
```

Hex, so 32 by 24. That these are the *whole* screen and not a corner of it is
settled by `AST2RD.MAC:1920`, which wraps a coordinate with `AND I,XRIGHT-1` —
a modulo that only makes sense across a full width — and by the hysteresis
comparisons against `XRIGHT/2` and `YTOP/2` at `AST2RD.MAC:2043-2061`, which
treat those halves as the distance to the edge.

512/32 and 384/24 are both exactly **16 generator units per playfield unit**,
and 512:384 is 4:3. Two independent sources, one written by the hardware people
and one by the game programmers, agreeing on both the scale and the aspect.

### 14.3 Measured against attract mode

Twenty frames of attract, exported through `tests/trace_sd.rs`:

```text
drawn extent    x -226..242, y -182..182   (full scale 256 by 192)
```

94% of the width and 95% of the height, exceeding neither. A title screen that
nearly fills the tube and stays inside it is what these constants predict.

### 14.4 What is still UNVERIFIED

- **What lies beyond.** Overscan, the DAC's own clip limits, and where the
  tube's edge actually falls are analogue questions the source cannot answer.
  512 by 384 is the deflection the *software drives*, which is what a beam
  trace needs. It is not proof that nothing exists outside it.
- **Bit 3 of the colour field.** The colour table (`AST2RD.MAC:244-251`) is
  three bits — bit 0 blue, bit 1 green, bit 2 red — and no attract-mode word
  has been seen to set the fourth.
- **Beam speed.** Nothing in the corpus times a vector. `BeamMove::ticks`
  models the cost of a move as its Chebyshev length and says so.
- **Intensity as light.** The 0-7 code's mapping to beam current, and thence to
  brightness, is not in the source.

### 14.5 Orientation, checked by looking

The beam trace format is y-up with the origin at the centre. Rendering the
exported attract trace puts "1 COIN 1 PLAYER" above the title, the title above
the play area, and the copyright line at the bottom, with no text mirrored —
so our y sign is right. The picture is 4:3 and the renderer's current tube
profile is a portrait Vectrex, so it is displayed in the wrong aspect until a
Wells-Gardner 6100 profile exists. That is the renderer's parameter file, not
the trace: aspect ratio is deliberately not encoded in a trace at all.

### 14.6 Intensity as light (**reasoned, not measured**)

`vg::intensity_drive` turns the three-bit code into radiant drive for a beam
trace, which wants **light per unit time** — so that energy is drive multiplied
by dwell — rather than a code or a voltage.

**The code is three bits, corroborated twice.** `VGUTR2.MAC:67-72`: "`VGBRIT`
... ITS VALUES ARE 0,10,20,30,40,...F0 WHERE 0 IS OFF AND F0 IS MAX BRIGHTNESS.
IN THE VECTOR INSTRUCTIONS ONLY THE UPPER 3 BITS IS USED." Independently,
`AST2RD.MAC:272` labels the variable "VECTOR BRIGHTNESS (0=OFF, F0=MAX, 20
INC)" — increments of `20` hex, which is eight steps, which is the three-bit
field exactly. The game's own use fits: `.BRITE =5` is its default
(`AST2RD.MAC:642`) and `ORA #0E0 ;FULL BRIGHT ON FLASH` (`AST2RD.MAC:731`) is
code 7. Twenty frames of attract use codes 5, 6 and 7 and nothing else.

**What a code does to a beam is not in the corpus, and the model is reasoning
rather than measurement.** The chain is a small weighted-resistor DAC into a Z
amplifier into a gun. Two things hold of that chain in general: the DAC is
linear in *voltage* by construction, and a CRT gun is not linear in *light* —
its output follows a power law of drive. So the model is
`(code/7) ^ INTENSITY_GAMMA` with the exponent at 2.2, the usual CRT figure.
It gives 0.48, 0.71 and 1.00 for the three codes the game actually uses.

Sources consulted and what they gave: Jed Margolin's ["The Secret Life of
Vector Generators"](https://www.jmargolin.com/vgens/vgens.htm) documents the X
and Y integrators and does not cover the Z path at all, so it does not settle
this. The [Wells-Gardner 6100 FAQ](https://www.vectorlist.org/Documents/6100_faq.pdf)
describes the monitor's Z amplifiers but was not reachable to quote.

**UNVERIFIED.** The shape is right on general principles; the exponent is a
starting value. Correcting it needs a board and a photometer, not a listing.

Checked by rendering, since ordering is the property that matters: three
identical strokes at the same screen position, one per code, through the tube
model, measure 10, 11 and 12 of 255 for codes 5, 6 and 7 — monotonic, all
visible, none saturating. Two earlier attempts at this measurement were wrong
and are worth recording so they are not repeated: strokes placed at *different*
heights are confounded by the vignette, which varies with position, and a high
exposure pushes all three into the flat top of the tonemap where they read
identically.
