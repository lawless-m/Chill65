# The Phase 3 differential harness

`chill65-diff` drives our runtime and one or more external reference
implementations from the same recorded input, compares their pictures frame by
frame, and names the routine behind the first disagreement.

## 1. It is a debugger, not a certifier

The plan is explicit about what this is for
(`atari-recompiler-plan.md:129`):

> Purpose is debugging, not certification: the useful output is "diverged at
> frame 4,812, routine X".

That wording decides the whole design, and in particular it rules out the gate
one would reach for first. **The harness does not assert that our runtime agrees
with MAME**, because it does not and should not. `gate2.md` records six claims
about the hardware that are explicitly marked UNVERIFIED:

1. `OUT1` bit 6 — the RTL says `STARTLED1`, the game source says "bothram".
2. Bitmap rows 24–31 are displayed but not writable through the coordinate
   window, while the game's own equate claims Y starts at 24.
3. The exact phase of the scroll counters — the residue of open question O4.
4. Cocktail flip: the latch bit is stored, the flip is not modelled.
5. Host-delta to trackball-count scaling.
6. Which CRAM entries a bitmap pixel selects is *derived* from the arbitration
   logic rather than measured, and motion objects are not modelled at all.

Against any external oracle, divergence is therefore **expected output**, not
failure. A gate demanding frame equality would be one the project cannot pass,
and an unattended loop would grind on it forever. What the harness is gated on
instead is stated in §8.

## 2. Trace format, version 1

`crates/chill65-diff/src/trace.rs`. UTF-8 text; `#` starts a comment to end of
line; blank and comment-only lines are ignored everywhere. The first line with
content is the header `chill65-trace 1`. Every line after it is one frame:

```text
<dx> <dy> <switches> [x<count>]
```

- `dx`, `dy` — decimal `i32` trackball deltas, positive right and down, passed
  to `Input::add_trackball_delta` unchanged.
- `<switches>` — `-`, or a comma-separated subset of `Start1`, `Start2`,
  `SelfTest`, `Slam`, `CoinAux`, `CoinLeft`, `CoinRight`. These are the
  `chill65_runtime::Switch` variants, case-sensitive. Listing a switch means it
  is **held** for that frame: levels, not edges, so a ten-frame press appears on
  all ten lines.
- `x<count>` — repeats that line `count` times in total, which is what keeps a
  600-frame idle trace to one line.

Switches are serialised in a canonical order, so a trace round-trips to
identical text. The frame count is the expanded line count.

## 3. The seam, and the canonical frame

`crates/chill65-diff/src/reference.rs`. Every oracle implements one trait:

```rust
fn run(&mut self, trace: &Trace, frames: u32, want_pixels: bool)
    -> Result<Vec<Frame>, String>
```

Frame `k` takes its input from `trace.frames[k]`, or idle input where the trace
is shorter than the run. A run must be deterministic — the same trace twice must
give the same hashes, or nothing built on top means anything.

Comparison happens in **256×232 RGB24, row-major**, three bytes per pixel
(`FRAME_BYTES`). That is the one form every implementation can produce.

This is why the harness does **not** reuse `Machine::frame_hash`. That hashes
palette *indices*, which exist only inside our machine model: MAME hands out
finished pixels and the RTL emits RGB straight off the video bus, and neither
can say which colour RAM entry a pixel came from. The harness hashes canonical
RGB instead, with the same FNV-1a the runtime already uses
(`chill65-runtime/src/video.rs:298`). Index-to-RGB conversion follows
`write_ppm` in `ccrun.rs` exactly: `BITMAP_CRAM_BASE` then `cram_rgb`.

## 4. Divergence, and blaming a routine

`diverge.rs` reports the **first** frame where two hash streams stop agreeing,
and only the first: everything after is downstream of that one divergence.
Streams of unequal length are compared over their common prefix and the mismatch
is stated rather than truncated away — two runs that produced different frame
counts have already gone wrong, and reporting that as a clean "identical" would
be the worst available answer.

Attribution rests on an optional log in the runtime
(`chill65-runtime/src/machine.rs`, `enable_write_log`). Off by default,
allocating nothing until asked; when on, every store that reaches bitmap RAM
records the PC of the instruction that made it, learned through a no-op
`Bus::begin_instruction` hook that `FlatBus` ignores.

**The log is keyed by RAM nibble, not by screen position.** A store lands at an
address, but which pixel of the picture that address *is* depends on the scroll
registers at the moment the picture is read. Recording screen positions at write
time would be silently wrong the first time the game scrolls, and Crystal
Castles scrolls. `Machine::pixel_writers` projects the log onto screen pixels on
demand, using the same mapping `framebuffer` uses.

`localise.rs` then takes both pictures at the divergent frame, asks the log which
instruction last wrote each differing pixel, maps it to the nearest preceding
symbol, and ranks. Blame aggregates **by routine**, not by instruction: the
attract screen's border alone is drawn from four consecutive addresses, and
per-instruction buckets bury the answer.

Two consequences worth stating:

- **The bitmap is never cleared**, so a pixel differing at frame 400 may have
  been drawn at frame 12 and left alone since. The log runs for the whole run
  and remembers the last writer however long ago that was.
- **Both sides can be blamed.** Under fault injection both runs are ours, and
  the interesting answer is sometimes "the clean side drew something here the
  faulty side never did". Against MAME or the RTL there is no log to merge, and
  attribution comes from our side alone — which still names the routine that
  drew *our* version of the disputed pixels.

Pixels and logs are captured in a second pass that runs only as far as the
divergent frame. Carrying them for every frame of a 600-frame run is 140 MB to
no purpose.

Limits, plainly: attribution can only name a routine that wrote through the CPU.
A pixel that differs because of something the video hardware did — scroll phase,
colour arbitration, an unmodelled motion object — has no writer to blame and is
counted as unattributed rather than pinned on whoever last touched that address.

## 5. Commands

```text
ccdiff run --trace T [--frames N] --out DIR [--pixels] [--prog P --data D]
ccdiff compare STREAM_A STREAM_B
ccdiff localise --trace T --sym S --prog-a P --data-a D --prog-b P --data-b D
ccrom [--corpus DIR]
```

`compare` exits 0 whether or not the streams diverge — a divergence is a result,
not an error — and nonzero only on an I/O or format fault. Everything written
goes under `target/`.

## 6. The ROM set

`romset.rs` rebuilds MAME's `ccastles3` from material the corpus already holds.
All eleven devices match MAME's own CRC-32, so its audit passes with no checksum
bypass and nothing is downloaded:

| Device | Bytes | Source | CRC-32 |
|---|---|---|---|
| `136022-303.1k` | 8192 | `prog.bin[0000..2000]` | `10E39FCE` |
| `136022-304.1l` | 8192 | `prog.bin[2000..4000]` | `74510F72` |
| `136022-305.1n` | 8192 | `prog.bin[4000..6000]` | `9418CF8A` |
| `136022-102.1h` | 8192 | `data.bin[0000..2000]` | `F6CCFBD4` |
| `136022-101.1f` | 8192 | `data.bin[2000..4000]` | `E2E17236` |
| `136022-106.8d` | 8192 | `372BR.RS4[0000..2000]` | `9D1D89FC` |
| `136022-107.8b` | 8192 | `372BR.RS4[2000..4000]` | `39960B7D` |
| `82s129-136022-108.7k` | 256 | `version-2/TPSYNC.LDA` | `6ED31E3B` |
| `82s129-136022-109.6l` | 256 | `version-2/TPBUS.LDA` | `B3515F1A` |
| `82s129-136022-110.11l` | 256 | `version-2/TOPOWP.LDA` | `068BDC7E` |
| `82s129-136022-111.10k` | 256 | `version-2/TPI.LDA` | `C29C18D9` |

The PROMs come from `version-2` deliberately: the root tree's four are damaged
and parse to nothing (`integrity.md` §7). `TPI.LDA` records only 128 bytes and is
padded to 256, which is exactly how MAME's dump of the same part reads.

**Why `ccastles3` and not the parent.** `ccastles` is revision 4
(`136022-403/404/405`) and no tree in the corpus can build it — root gives
103/104/105, `version-2` gives 203/204/205, `version-3` gives 303/304/305
(`integrity.md` §6 and §8). Revision 3 is what our assembler produces and
therefore what `chill65-runtime` executes; driving an oracle on any other
revision would manufacture divergences that mean nothing.

**The filename trap.** MAME locates a set by **zip filename**. The archive must
be `ccastles3.zip`, however tidy another name might look.

CRC-32, the ZIP writer and the `.LDA` decoder are hand-rolled — the workspace has
no third-party crates, deliberately. The archive is deterministic: timestamps
are fixed rather than read from the clock, because an artefact that changes on
every rebuild can be made but not checked.

## 7. The trace corpus

`traces/` holds five committed traces. **These are committable**: plan §9
(`atari-recompiler-plan.md:197`) forbids ROM images, framebuffers and the
`.MAC`/`.LDA`/`.DAT` source; a recording of what someone did with the controls is
our own authored work and none of those things. Every file says so in its header.

| Trace | Frames | Differs from idle at |
|---|---|---|
| `idle-attract.trace` | 600 | — (baseline) |
| `selftest.trace` | 600 | 366 |
| `coin-start.trace` | 900 | 403 |
| `gameplay.trace` | 2640 | 403 |
| `wrap-stress.trace` | 2006 | 403 |

Timings come from the game, not from guesswork. The coin sequence follows
`CCN.MAC:269-300`, Mike Albaugh's routine: the switch is sampled **four times a
frame**, a valid coin is 16–800 ms of contact — so a longer press is not a safer
one, and past 800 ms it is rejected as too long — and the credit arrives only 30
frames later, once `$PSTSL`, the post-coin slam timer, has run down. A start
press before that does nothing at all.

`wrap-stress.trace` exists because `SupportChips.v:297-307` is a free-running
9-bit counter with no saturation and no reset, exposed as `count = counter[8:1]`.
It reports absolute position and wraps; the game differences it itself
(`CEN.MAC:317-330`).

`selftest.trace` parts from idle at frame **366**, near `hardware.md` §10.2's
placing of the end of the power-on self-test around frame 353. (It read 354
before colour RAM was initialised to match the oracles — see §11.3 — since the
harness hashes colours and the first frames are no longer uniform.)

## 8. What the harness is gated on

Not agreement with anything external. The gate is that **the machinery works**:
given a difference introduced in a known place, it finds the right frame and
names the right routine, reproducibly; given identical runs it reports nothing.

The fixture took some finding, and the obvious version of it is wrong. Flipping
one bit in a drawing routine does not test what it appears to: the game
checksums its own ROMs at power-on, so *any* altered program byte is caught and
every such fault yields the same picture — the self-test's failure screen.
Measured, that is a divergence at frame 371 with 56,836 of 59,392 pixels
differing, all attributed to `BITEST`. The flipped routine never gets to draw
anything wrong. Two entirely different bit flips gave byte-identical reports,
which is what gave it away.

The checksum is a longitudinal parity — `EOR NY,TEMP1` at `CST.MAC:331` — so
flipping the same bit in **two** bytes of one 8K device leaves it unchanged. The
fixture flips bit 0 at `A408` and `A418`, both inside `LN.F1`, both in
`136022-303.1k`:

- `A418` is the `32` of `LDA $32`, the zero-page byte holding the X coordinate.
  Flipped, the routine reads `$33` — the Y coordinate — and draws in the wrong
  place. This is the fault.
- `A408` is the `00` of `LDA #$00` at `LN.F1+1`, whose value reaches only the
  OUT1 latch. That latch samples **D3 only** (`CCastles.v:332`), and bit 3 of
  `00` and `01` agree, so the flip changes nothing observable. It exists purely
  to restore the parity.

One flip does the damage, the other is invisible, and together they satisfy the
game's own diagnostic. The result:

```text
clean vs clean:  identical over 600 frames
clean vs faulty: diverged at frame 419: LN.F1 [CRF.MAC] (85 pixels),
                 SQ.LDR [CRF.MAC] (50 pixels), WV.BDR [CRF.MAC] (35 pixels)
```

`LN.F1` is the routine holding the flipped byte, named without being told, the
same way twice. The precedent for gating on a deliberate failure is
`chill65-runtime/tests/selftest.rs`, which proves the ROM self-test *fails* on a
corrupted byte rather than merely that it passes.

## 9. Environment

| Variable | Meaning |
|---|---|
| `CHILL65_CORPUS` | game source root; every test needing it is `#[ignore]`d |
| `CHILL65_KLAUS` | Klaus Dormann's 6502 suite, for the runtime's CPU tests |

The convention is that an unset variable makes its test **skip cleanly** — print
and return — never fail. See `images::corpus`.

## 10. MAME as an oracle

MAME 0.276, driven headless through its own Lua scripting API. Nothing modifies
MAME or reaches past the documented interface. Gated on `CHILL65_MAME`, the path
to the executable; unset means skip cleanly.

The screen is **256×232 at 61.035 Hz** — exactly the canonical frame, so no crop
and no scale. `screen:pixels()` returns packed 32-bit values, `B G R A` on this
host, alpha discarded. The capture script writes the geometry it actually saw to
a sidecar and the Rust side checks it, so a future MAME that changes it errors
rather than silently misaligning.

### 10.1 The discovered port map

Found at runtime by iterating `manager.machine.ioport.ports` rather than
assumed:

| Field | Port | Mask | Our `Switch` |
|---|---|---|---|
| `Right Jump/2P Start Upright` | `:IN0` | `0x80` | `Start2` |
| `Left Jump/1P Start Upright` | `:IN0` | `0x40` | `Start1` |
| `Service Mode` | `:IN0` | `0x10` | `SelfTest` |
| `Tilt` | `:IN0` | `0x08` | `Slam` |
| `Service 1` | `:IN0` | `0x04` | `CoinAux` |
| `Coin 1` | `:IN0` | `0x02` | `CoinLeft` |
| `Coin 2` | `:IN0` | `0x01` | `CoinRight` |
| `Trackball Y` | `:LETA0` | `0xFF` | vertical axis |
| `Trackball X` | `:LETA1` | `0xFF` | horizontal axis |

Every mask lands on exactly the bit `Switch::bit` names, and LETA0 is vertical
with LETA1 horizontal just as `input.rs` says from `CCastles.v:292` and the
game's `HW.TBV`/`HW.TBH` equates. That is a third independent witness to the
input model, arrived at from MAME's driver rather than from the RTL or the game
source.

Several fields share a mask — the upright and cocktail namings of one physical
input — so the script keys by mask rather than by name and never has to choose
between them arbitrarily.

### 10.2 The two translations

**Switches become an `IN0` mask**, bit for bit, per the table above.

**Deltas become positions.** Our traces record per-frame movement, but the LETA
reports an *absolute* count which the game differences itself (`CEN.MAC:317`,
`TR.DEL`). The harness therefore accumulates the trace deltas and hands MAME the
running total modulo 256. One trace unit is one CPU-visible count. `input.rs`
marks host-delta-to-count scaling as UNVERIFIED; this is that choice made
explicit rather than buried.

### 10.3 Measured

```text
mame determinism: 500 frames identical across two runs
mame input:       coin-start differs from idle at frame 402
ours vs mame:     diverged at frame 0: 59392 pixels differ, none attributable
```

Determinism holds with fresh `cfg` and `nvram` directories per run. Input lands:
`coin-start.trace` parts from idle at MAME frame 402, against 403 on our side —
a one-frame offset, unexplained and worth returning to.

### 10.4 The first divergence — resolved, and what was behind it

This first read **frame 0, every pixel, nothing attributable**: our frame 0 was
uniformly white, MAME's uniformly black. The cause was the reset state of colour
RAM, diagnosed in §11.3 and since **fixed** — `Video::new` now sets entry 16 to
`0x1FF`, so the machine powers on black as both oracles do.

That divergence had nothing to do with drawing and masked everything after it.
With it gone, the comparison says something:

```text
ours vs mame: diverged at frame 162: MN.ST [CRF.MAC] (2 pixels)
```

Both implementations now agree for **161 consecutive frames**, and the first
difference is **two pixels** at frame 162 — the exact frame `boot.rs` measures as
our first draw — attributed to `MN.ST`.

The verilated MiSTer core, entirely independently, reports **the same frame, the
same routine and the same two pixels** (§11.7). Two oracles built by different
people from different sources converging on one answer looked at the time like
strong evidence of a real difference in our model.

> **Superseded by §12.** Both claims in this subsection are weaker than they
> read. The 161 frames of agreement are 161 frames in which *both sides are
> blank*, which agree at any alignment and carry no information. And the two
> pixels are the power-on RAM test's cursor, sampled at different phases by each
> implementation — the oracles catch it too, on other frames. §12 has the
> measurements.

Unresolved and now worth chasing: what those two pixels are. `MN.ST` is a
drawing routine; the difference appears at the very first frame anything is
drawn, which suggests an edge condition rather than a systematic error.

## 11. The verilated MiSTer core

Verilator 5.032 against `Arcade-CrystalCastles_MiSTer`, which is third-party
reference material and is **never modified**. Verilator 5 is stricter than the
4.x era that core was written against, so every accommodation is a flag
(`tools/build-mister-sim.sh`):

- `--no-timing` — the RTL uses `#1` delays throughout, a simulation nicety
  rather than behaviour, and verilator 5 refuses to guess.
- `-Wno-fatal` — lint findings are reported but do not stop the build. They
  concern code that synthesises perfectly well for the FPGA the core targets.

Both source directories are passed with `-y`, and every `rtl/*.v` except the
Altera-specific `pll.v` is named explicitly, because several modules — `sram`
among them — live in files not named after them.

### 11.1 The download map

The core takes ROMs through `dn_clk`/`dn_wr`/`dn_addr`/`dn_data`, with
`dn_addr[15:13]` selecting one of seven 8K devices (`ProgramMemory.v`,
`MotionObjectPictureRom.v`):

| `dn_addr[15:13]` | Device | Our member |
|---|---|---|
| 0 | ic1F | `136022-101.1f` |
| 1 | ic1H | `136022-102.1h` |
| 2 | ic8D | `136022-106.8d` |
| 3 | ic8B | `136022-107.8b` |
| 4 | ic1K | `136022-303.1k` |
| 5 | ic1L | `136022-304.1l` |
| 6 | ic1N | `136022-305.1n` |

That is exactly the part order of the core's own `.mra`, so the blob is those
seven members concatenated. The four PROMs are not downloaded; the core does not
model them.

### 11.2 The observable window is 252 pixels, not 256

The top-level `HBLANK` is `HBLANK2`, the *delayed* blanking — set at hcount 259,
cleared at hcount 7 (`SyncChain.v:35-38`) — and `RGBout` is forced to zero
whenever it is asserted (`CCastles.v:353`). So the core emits **252 pixels per
line**. The first columns are blanked at the port and cannot be observed from
outside at all.

This is a property of the core's interface, not something a flag fixes. The
simulation reports the widest active line it saw and the harness records it, so
any comparison must confine itself to the columns the core actually emits rather
than pretending the rest are black. Colour is expanded from `RGBout[8:6]`,
`[5:3]`, `[2:0]` with the same `v * 255 / 7` scaling `cram_rgb` uses, so the two
implementations are directly comparable.

### 11.3 Colour RAM at power-on — our runtime is the outlier

`ColorMemory.v` initialises colour RAM through a relative `$readmem` of
`cram.rom`, so the simulation's **working directory** decides whether the core
powers on with its intended palette. Run from elsewhere, verilator warns and
leaves CRAM zeroed.

That file is 32 entries, all `000` **except entry 16, which is `1FF`**. Entry 16
is `BITMAP_CRAM_BASE` — the entry a bitmap pixel of zero selects — and `1FF`
inverts to black. So the core powers on with a **black** background.

Measured, all three implementations from cold:

| Implementation | Frame 0 |
|---|---|
| MAME | black |
| MiSTer core, `cram.rom` loaded | black |
| MiSTer core, `cram.rom` missing | white (an artefact, not the core's behaviour) |
| **`chill65-runtime`**, as measured | **white** |

`Video::new` set `cram: [0; 32]`, and a zeroed entry 16 decodes to full white.
So on this point **our reset state disagreed with both external oracles**, and
that — not anything about drawing — was what §10.4's frame-0 divergence actually
was.

**Decision taken: match the oracles.** `Video::new` now initialises entry
`BITMAP_CRAM_BASE` to `0x1FF`, so the machine powers on black. On real hardware
colour RAM is RAM with undefined power-on contents, so this is not the correction
of a proven error — it is a deliberate choice to agree with two independent
implementations, made because the disagreement was masking every later
divergence. A comparison window was the alternative and was not taken: changing
the reset state costs one line and leaves the harness measuring whole runs.

`Machine::frame_hash` hashes palette *indices*, so nothing in Phase 2 moved:
attract is still `087e02f4ca003874` at 12,288,528 cycles and the self-test
verdict is unchanged. The harness hashes colours, so its streams did move —
`selftest.trace` now parts from idle at frame 366 rather than 354 — and the
fault-injection fixture is unaffected, still frame 419 and `LN.F1`, because both
sides of an ours-versus-ours comparison shifted together.

What the change bought is in §10.4 and §11.7: both external comparisons went
from an unattributable wall at frame 0 to a two-pixel difference at frame 162.

### 11.4 Measured

The simulation is deterministic: two runs byte-identical. It costs roughly half
a second of wall clock per emulated frame, so it is used sparingly.

### 11.5 Boot timing: the core draws, later than we do

The question left open above is answered. Run to 500 frames, the core draws:

| Frame | Lit pixels |
|---|---|
| 0–150 | 0 |
| 200 | 2 |
| 350 | 176 |
| 400 | 17,024 |
| 450 | 32,573 |

Our runtime's own `boot.rs` measures first draw at frame **162** and 33,238 lit
pixels by frame 600. The core reaches a comparable picture, roughly **200 frames
later**. Whether that is boot timing, a reset-release difference, or an offset
in what each side counts as frame zero is not established — but the core is not
broken, and the earlier reading of "barely draws" was a test that stopped at 240.

### 11.6 Trackball injection

The core takes quadrature, not counts. `SupportChips.v:297-307` is a
free-running 9-bit counter exposing `count = counter[8:1]`, so **two quadrature
half-steps make one CPU-visible count**. The simulation therefore emits two
half-steps per unit of trace movement, at most one per pixel clock, so a frame's
worth of motion is spread across the frame rather than delivered as a burst.

The input file is byte-for-byte the one MAME is given — `<IN0 mask> <x> <y>` per
frame, an absolute position rather than a delta — so a single serialiser feeds
both oracles and the two adapters cannot drift apart.

Which line of each quadrature pair leads is **not established**. `CCastles.v:292`
wires `.X2(tb1HD), .Y2(tb1HC)` horizontally and `.X1(tb1VD), .Y1(tb1VC)`
vertically; the harness treats the first of each pair as A. If that is backwards
the axis simply runs the wrong way, which would show as inverted motion. Recorded
rather than asserted, alongside the host-delta scaling `input.rs` already marks
UNVERIFIED.

### 11.7 Measured, and what it is worth

```text
mister determinism: 60 frames identical across two runs
ours vs mister:     diverged at frame 162: MN.ST [CRF.MAC] (2 pixels)
```

The simulation is deterministic across genuine re-simulations, not merely
replays.

The comparison first read `diverged at frame 0: 59392 pixels differ, none
attributable` — the same wall MAME hit, and for the same reason: our white
against the core's black. With colour RAM initialised to match (§11.3), it now
agrees with us for 161 frames and then differs by **two pixels** in `MN.ST`.

**MAME, independently, reports the identical frame, routine and pixel count**
(§10.4). Two oracles with nothing in common but the hardware they model — one a
C++ emulator, one an FPGA core verilated from Verilog — converging on the same
two pixels is about as strong as this kind of evidence gets. Whatever those two
pixels are, they are ours.

Bounds on any comparison against this core, for whoever acts on it:

- it emits **252 of 256 columns**, and the rest cannot be observed at its ports;
- its picture arrives roughly 200 frames later than ours;
- attribution comes from our side alone, since the core has no write log.

## 12. The two pixels at frame 162, explained

§10.4 and §11.7 record MAME and the verilated MiSTer core independently
reporting the same two-pixel difference at frame 162 in `MN.ST`, and conclude
"whatever those two pixels are, they are ours". They are ours. They are also
**not a fault in the machine model** — they are an artefact of how this harness
extracts a picture, and the same artefact will recur wherever the game writes a
transient value into bitmap RAM.

### What is drawing them

`MN.ST` is the power-on entry, and `EBDE`–`EBFA` is Atari's RAM test:

```text
EBDE  LDX #1              ; "test cleared memory"
EBE5  LDA (TEMP1),Y       ; "STILL ZERO?"
EBE7  BNE RAMERR
EBE9  LDA #FF
EBEB  STA (TEMP1),Y       ; write FF
EBED  EOR (TEMP1),Y       ; read back; must cancel to zero
EBEF  BNE RAMERR
EBF1  STA (TEMP1),Y       ; write the zero back
EBF3  INY / BNE           ; next byte
EBFA  CPX #90             ; pages 01-8F
```

It walks every byte from `0100` to `8FFF`, and **the bitmap is in that RAM**
(`machine.rs`: *"0000-7FFF RAM — and the bitmap"*). A byte of `FF` is two
nibbles of `0x0F`, selecting colour RAM entry 31 — which is zero, which decodes
to **white**, because entries are active-low (§on `cram_rgb`).

So each byte under test is two white pixels for the ~13 cycles between the store
at `EBEB` and the store at `EBF1`, out of roughly 34 cycles per byte.

The write log names `EBEB` as the writer of both pixels, and the lit byte
**walks through memory** exactly as the test does:

```text
frame 162  (122,  2)      frame 174  (142, 60)
frame 164  ( 40, 12)      frame 176  ( 60, 70)
frame 168  (132, 31)      frame 180  (152, 89)
frame 170  ( 50, 41)      frame 182  ( 70, 99)
frame 172  (224, 50)      frame 184  (244,108)
```

Always exactly two pixels, always one byte, advancing steadily until the test
finishes around frame 208. Twenty of the 140 frames from 120 to 260 show it.

### Why the oracles disagree about *which* frames show it

The first version of this section claimed the oracles essentially never catch
the cursor, because a beam reads each byte in well under a cycle while the byte
is `FF` for only thirteen. **That is wrong, and measuring it says so.** MAME
catches the cursor about as often as we do — it simply catches it on *different
frames*:

```text
frame   ours-lit  mame-lit          frame   ours-lit  mame-lit
  162          2         0            182          2         2
  164          2         0            183          0         2
  166          0         2            184          2         0
  168          2         2            185          0         2
  170          2         0            187          0         2
  176          2         2            188          2         2
  178          0         2            195          0         2
```

Both are sampling one marker that moves through memory faster than the frame
rate. Which frames catch it depends on the phase of each implementation's
sampling instant against the test's ~34-cycle loop, and the two phases are not
the same. Frame 162 is not where the machines diverge; it is simply **the first
index at which our sample caught the cursor and theirs did not**.

That is aliasing, and it is not fixable by making the model more correct. Every
difference from frame 162 to about frame 208 has this character.

### The frame numbers are not aligned

The finding above is a symptom of something the harness does not establish:
**that our frame *N* and an oracle's frame *N* are the same frame.** The
comparison lines them up by index and reports the first index that differs.
Three measurements say that assumption is unsafe.

**The 161 frames of prior agreement are vacuous.** §10.4 says the two "agree for
161 consecutive frames" before frame 162. They do — and both are *blank* for
every one of them. Measured directly: frames 0–161 have zero lit pixels on both
sides. Two blank screens agree at any offset, so those 161 frames carry no
alignment information at all.

**The oracle draws later, and draws partially.** Where the picture gains
content, ours changes a frame or two before MAME's, and MAME shows a frame
caught *mid-draw* that we never do:

```text
frame   ours-lit  mame-lit
  312         88         0
  313         88        31      <- MAME, mid-draw
  314         88        88
  335        176        88
  336        176       176
```

Our snapshot is all-or-nothing: the frame boundary falls between the writes, so
a picture is either drawn or not. MAME's 31-of-88 at frame 313 is the beam
passing through a picture while the game is still drawing it, which is what real
hardware does and what our extraction cannot represent.

**A shift search does not find a clean offset.** Counting exact frame matches
over frames 200–420 at shifts of −4 to +4 gives 154, 154, 157, 158, 161, 163,
163, 159, 157. The best shifts are +1 and +2 rather than 0 — but by two frames
out of 220, which is noise. Much of that range is static, so most frames match
at any shift; the test cannot discriminate.

The existing one-frame coin offset (§10.3: MAME's `coin-start` differs from idle
at frame 402, ours at 403) is the same phenomenon seen from another angle.

**What this means for reading a divergence report.** A reported frame number is
the first index where two streams differ, which conflates a genuine difference,
an offset between the two frame numberings, and one side catching a picture
mid-draw. It is reliable for *static* content and worth no more than ±2 frames
for anything changing. Nothing in the harness is calibrated against a shared
timebase, and until something is, "diverged at frame N" should be read as
"differences begin around frame N".

This does not weaken the fault-injection gate, which localises a *routine* from
a persistent difference in a static picture (§8). It does weaken any inference
from a small divergence in a frame where something is being drawn.

### What was nearly changed, and should not be

The obvious first hypothesis is that colour RAM entries 17–31 should power on
black like entry 16, since they are the ones decoding to white. **The MiSTer
core says otherwise.** `rtl/cram.rom` is ASCII, and reads:

```text
000 000 000 000 000 000 000 000 000 000 000 000 000 000 000 000
1FF 000 000 000 000 000 000 000 000 000 000 000 000 000 000 000
```

Entry 16 is `1FF`; entries 17–31 are `000`, exactly as `Video::new` leaves them.
Ours already matches the reference implementation, and "fixing" those entries
would have moved us away from it while appearing to fix the symptom.

### The general caveat

**Any transient the game writes into bitmap RAM appears in our snapshots and not
on hardware.** The RAM test is the loudest case because it touches every byte,
but it is a property of the extraction method, not of that routine. Anything
comparing our pictures against a scanning implementation inherits it.

Modelling scanout timing would remove it and is not proposed: the snapshot is
deliberate, it is what makes a frame hash cheap and deterministic, and the error
it admits is bounded by how long a byte spends holding a value it is about to
lose.

`gate3.md` and `gate4.md` record this divergence as unexplained, which it was
when they were written.
