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
| `selftest.trace` | 600 | 354 |
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

`selftest.trace` parting from idle at frame **354** is independent corroboration
of `hardware.md` §10.2, which puts the end of the power-on self-test around
frame 353.

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

### 10.4 The first divergence, and what it actually is

Frame 0, every pixel, nothing attributable. Diagnosed rather than left as a
number:

- our frame 0 is **uniformly white** (255,255,255), one distinct colour;
- MAME's frame 0 is **uniformly black**, one distinct colour.

The cause is the reset state of colour RAM. Ours is all zeros, and `cram_rgb`
inverts every component — `o = {~rbg[8:6], ~rbg[2:0], ~rbg[5:3]}`,
`ColorMemory.v:33` — so a zeroed CRAM decodes to full white. MAME starts black.

On real hardware colour RAM is RAM, and its power-on contents are undefined, so
neither is obviously wrong; the game writes CRAM before it draws anything. But
the consequence for the harness is concrete: **the very first frame diverges for
a reason that has nothing to do with drawing, and masks everything after it.**
The report is accurate and useless in equal measure.

The refinement this asks for is a comparison window — start at a frame after the
game has written CRAM, the way `idle-attract.trace` already waits out the
self-test. That is recorded here rather than done, because it changes what the
gate measures and deserves its own decision.

## 11. The MiSTer RTL oracle

Not yet built. Verilator 5.032 is installed. This section will carry the
`dn_addr` download map, the quadrature scaling decision, the RGB expansion, and
the first observed divergence with its attribution.
