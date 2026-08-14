# Phase 2 gate — PASSED

The runtime runs Crystal Castles from source assembled by our own toolchain, and
the game's own diagnostic agrees the bytes are right.

**Space Duel now clears the same bar** — see §"Space Duel" below. Everything
above and before that section is the original Crystal Castles gate, unchanged.

## The five commands, one run, all exit 0

| # | Command | Result |
|---|---|---|
| 1 | `cargo build` | clean |
| 2 | `cargo test` (no corpus, no network) | **210 passed**, 0 failed |
| 3 | `cargo test -p chill65-asm --test gate -- --ignored` | **byte-identical, both images** |
| 4 | `cargo test -p chill65-runtime -- --ignored` | **6 passed** across attract, boot, klaus, selftest |
| 5 | `cargo test -p chill65-runtime --test klaus -- --ignored` | **2 passed** |

### 3 — Phase 1 non-regression

```text
PROGRAM: 24576 of 24576 bytes match (100.00%), 0 differ
DATA:    16384 of 16384 bytes match (100.00%), 0 differ
```

Unregressed. Nothing in Phase 2 touched `chill65-asm` behaviour.

### 4 — the runtime on the real ROMs

```text
self-test: ROMTST at 6,451,839 cycles, GOTCHK at 6,866,406
           (414,567 cycles scanning 40K across both banks)
           verdict Passed { checksums: [1, 2, 3, 4, 5] }
corrupted ROM correctly rejected: checksums [00, 02, 03, 04, 05]

booted:    420 frames, 8,601,893 cycles, 200 interrupts taken
           first draw frame 162, first interrupt frame 353
           87 distinct images, 28,681 lit pixels

attract:   600 frames, 12,288,528 cycles, hash 087e02f4ca003874
           33,238 lit pixels — identical across independent runs
```

### 5 — CPU validation

```text
Klaus decimal test:    finished after 14,464,187 instructions, ERROR=0
Klaus functional test: parked at 3469 after 30,646,177 instructions
```

## Distribution posture — confirmed clean

`git status --porcelain` lists no game-derived artefacts. Nothing matching
`*.bin`, `*.ppm`, `*.LDA`, `*.MAC` or `*.DAT` is tracked. The three third-party
trees (`crystal-castles/`, `Arcade-CrystalCastles_MiSTer/`,
`6502_65C02_functional_tests/`) are gitignored and do not even appear as
untracked. Every generated image under `target/boot-artefacts/` is ignored.

Per plan §9: we ship the toolchain, the user supplies their own source.

## The deliverable

`hardware.md` — 763 lines, 12 sections, **75 `file:line` citations**, 17 facts
marked corroborated by two independent sources, and **6 claims explicitly marked
UNVERIFIED**. Required coverage all present: video (§5), the Potato chip as open
question **O4** (§5.6), timing (§7), inputs (§9).

The unverified six, kept honest because Phase 3's differential harness compares
against this document and a guess recorded as fact would be self-confirming:

1. OUT1 bit 6 — RTL says `STARTLED1`, the game source says "bothram" (§2.1).
2. Bitmap rows 24–31 are displayed but not writable through the coordinate
   window; the game's own equate claims Y ranges from 24 (§5.3).
3. The exact phase of the scroll counters — the residue of **O4** (§5.6).
4. Cocktail flip: the latch bit is stored, the flip is not modelled (§5.6).
5. Host-delta to trackball-count scaling (§9.5).
6. Which CRAM entries a bitmap pixel selects is *derived* from the arbitration
   logic, not measured; motion objects are not modelled (§12.1).

## What Phase 2 established

- A 6502 interpreter validated against 45 million instructions of Klaus
  Dormann's suite, including full decimal-mode flag semantics.
- A machine model — bus, ROM banking, bitmap coordinate window, colour RAM,
  frame timing, two POKEYs with a real poly17/poly9 LFSR, switches and
  trackball — every behaviour cited to RTL or game source.
- **Atari's own ROM self-test passes on all five devices across both banks**,
  and provably fails on a single corrupted byte.
- Deterministic execution, which Phase 3 depends on.
- A headless runner producing an inspectable picture that is recognisably
  Crystal Castles.

## Deliberately out of scope

Windowing and audio output, and any third-party crate for them. The core stays
dependency-free and headless. "Playable" is a subsequent human-judged milestone;
`ccrun --dump` provides the artefact for judging it.

## Space Duel — the same bar, a different machine

Space Duel boots, draws, passes its own self-test and runs attract mode
deterministically, on hardware that shares almost nothing with Crystal
Castles' but the CPU and the sound chip.

| Check | Result |
|---|---|
| `vg_lists` | 19 real display lists execute and terminate cleanly |
| `boot_sd` | boots, 797 interrupts, 102 `GOADD` strobes over 102 steady frames |
| `selftest_sd` | **7 ROM checksums and 4 error flags, all zero** |
| `attract_sd` | 1,200 frames, 825 distinct pictures, two runs identical |

```text
CHILL65_CORPUS=/path/to/space-duel \
  cargo test -p chill65-runtime --test vg_lists    -- --ignored --nocapture
  cargo test -p chill65-runtime --test boot_sd     -- --ignored --nocapture
  cargo test -p chill65-runtime --test selftest_sd -- --ignored --nocapture
  cargo test -p chill65-runtime --test attract_sd  -- --ignored --nocapture
```

### What is new, and where it came from

Crystal Castles draws into a bitmap. Space Duel has none: the CPU builds a
**display list** in vector RAM and a vector generator executes it. That model
(`crates/chill65-runtime/src/vg.rs`) is decoded from `space-duel/VGMC.MAC` —
Atari's own macros for "the auto-normalizing vector generator" — with
`VGUTR2.MAC`'s run-time library corroborating every word. The board around it
(`src/sd.rs`) comes from `AS2DEC.MAC`, and **MAME's `spacduel` ROM layout
independently corroborates the ROM half of that map**.

The two modules carry 74 source citations and 15 explicit `UNVERIFIED`
markers. `machine.rs`, `video.rs`, `frame.rs` and `input.rs` are untouched:
the `Bus` trait was already the seam that lets two boards differ, and bending
one struct to cover both would have made each harder to check against its own
evidence.

### The self-test is the strongest result

`AS2TST.MAC` holds `POWERON`, which *is* the reset vector, so its zero-page,
VG RAM, ROM, POKEY and EAROM checks run on every boot. Its ROM checksums are
EOR sums seeded by the `CKUM` bytes Atari planted through the source, so a
correct image sums to zero and `AS2TST.MAC:404-406` sounds an alarm otherwise.

All seven come out zero. The game checks the Phase 1 build with the original's
own arithmetic and is satisfied — a second, independent witness to the
byte-identical image, arriving from inside the game rather than from a
comparison against the `.LDA` oracle.

### Two things worth knowing

- **The boot is not instant.** The first interrupt lands on frame 0, but the
  first `GOADD` is frame 98: the quiet stretch *is* the power-on self-test. A
  smoke test that stopped inside it would see a blank screen and a healthy
  watchdog and be unable to tell that from a wedge.
- **A vector machine can wedge without crashing.** If the display list stops
  being rebuilt, the generator redraws the last one and the interrupt handler
  keeps feeding the watchdog. A frame hash that stops changing is the only
  signal, which is why `attract_sd` watches the picture evolve.

### Some shapes do not close, and that is not our bug

Draw the attract screen and a few corners visibly miss — most obviously on the
title cubes, where an edge stops short of its vertex. It is tempting to hunt
for a rounding error in the generator. Don't: the gaps are in Atari's own
coordinate tables.

`AS2CU2.DAT`'s `CUBE01` begins with a blank move to `(2,18)` and its hexagon
comes back round to `(0,18)` — two units adrift. Its three sibling rotation
frames, `CUBE11`, `CUBE21` and `CUBE31`, close exactly. That is verifiable by
reading the source text alone, with no assembler, generator or renderer in the
path, and our decode reproduces every delta of it. Since the assembled image is
byte-identical (§gate1), the display lists we execute *are* the shipped ones.
The cube spins, so the gap blinked in and out on the original machine too.

It is not confined to the attract screen — `SHIELD`, `COMET`, `DWARF`, the
rocks, pyramids and saucer all have dangling vertices. But at 1-3% of their own
segment lengths those sit far below the beam spot. `CUBE01` is an order of
magnitude worse in proportion, at 11%, and the title cubes are drawn large,
which is why it is the one you see.

Two traps in measuring this, both fallen into first time round:

- An **open end is not a defect**. Any shape drawn as a stroke rather than a
  closed loop has two loose ends by design; `SHOT` is four vectors making a
  tick mark. Only a gap that is *small relative to the shape's own segments*
  is a corner that missed.
- A **text scan of the `.DAT` files is not enough**, because shapes call
  sub-shapes with `JSRL` and a parser that ignores them computes the wrong
  geometry. Run the shapes through the generator instead.

Faithful reproduction means keeping these. If they are ever to be corrected it
should be a deliberate, separately flagged option, never a silent fix in the
decoder.

## Next

Phase 3 — the MAME differential harness. The plan is emphatic that it be built
**before** the emitter. That is a fresh decision, not a continuation.

For Space Duel the harness has a head start it did not have for Crystal
Castles: MAME ships a `spacduel` driver, the images are byte-identical, and
the runtime is already proven deterministic frame for frame.

## Tempest — the same bar, a third machine

Tempest boots, draws, passes its own diagnostic and runs attract mode
deterministically, on a board that adds two things neither earlier title has:
a colour vector generator and a second processor.

| Check | Result |
|---|---|
| `vg_lists_te` | 14 real display lists execute and terminate cleanly |
| `boot_te` | boots at frame 17, 3,447 interrupts, 3,447 `VGSTART` strobes, 365 segments |
| `selftest_te` | **12 ROM checksums and 5 condition flags, all zero** |
| `attract_te` | 1,200 frames, 751 distinct pictures, two runs identical |
| `roms_te` | both ROM builders agree on all 24,576 bytes |

```text
CHILL65_CORPUS=/path/to/tempest \
  cargo test -p chill65-runtime --test vg_lists_te -- --ignored --nocapture
  cargo test -p chill65-runtime --test boot_te     -- --ignored --nocapture
  cargo test -p chill65-runtime --test selftest_te -- --ignored --nocapture
  cargo test -p chill65-runtime --test attract_te  -- --ignored --nocapture
  cargo test -p chill65-runtime --test roms_te     -- --ignored --nocapture
```

### What is new, and where it came from

**The vector generator is the same machine.** `VGMC.MAC` is byte-identical
between the Space Duel and Tempest corpora — same md5 — so every opcode, the
five-level stack, the word addressing and the `ZZ` rules are shared verbatim.
Exactly one thing differs: the status word. `ALDISP.MAC:34-35` defines
`MZCOLO=8` — "NEW COLOR STAT BIT MASK" — against `MZBRIT=0`, and
`ALVGUT.MAC:224`'s `VGSTAT` ORs them into the high byte, so a colour word is
`68xx` and an intensity word `60xx`. Bit 11 *selects* which field the word
carries; `ALDISP.MAC:1182-1185` issues one of each back to back, which is the
proof, since a word carrying both would need only one. The fields overlap, so
combining them corrupts in both directions: `ALVROM.MAC:161-163`'s `CSTAT`
emits `68C0+colour`, whose `C0` reads as a bright intensity, and an intensity
word's `F0` reads as colour 0. `Vg::stat_decode` keeps Space Duel's behaviour
as the default, so that board is untouched by construction.

**The palette is part of the picture.** Sixteen entries at `COLPORT`
(`ALCOMN.MAC:241`), rewritten every frame by `ALDISP.MAC:1078-1100`.
`TeMachine::frame_hash` folds the colour RAM in beside the segments, because a
frame whose geometry is unchanged but whose colours moved is a different
picture — and leaving it out would blind the attract test's wedge detection to
exactly the animation Tempest is known for.

**The math box, modelled functionally.** `MBUDOC.DOC:23-73` documents all 32
addresses with per-operation cycle timings, and `mbox.rs` implements the
operations rather than executing `MBUCOD.V05`'s microcode, per
`atari-recompiler-plan.md` §6. Two questions the documentation does not settle
were settled by the game:

- **Nothing is ever busy.** No consumer in the corpus requires observing
  busy=1 — every one polls with a `BMI`-shaped loop that falls through. The
  binding constraint runs the other way: `ALTEST.MAC:784-799` allows 100 poll
  iterations before declaring a timeout, so an operation that took too long
  would fail the diagnostic while one completing instantly cannot.
- **The divide is unsigned.** `MBTEST` walks its operand upward through every
  16-bit value (`ALTEST.MAC:777-780`), so it eventually divides `8000` by
  `8000` and still demands 1. Two's complement would give -1 and fail the box.

**`5000` is two things at once.** `ALCOMN.MAC:267-268` makes `INTACK` the same
address as `WTCHDG`, so one write both feeds the watchdog and acknowledges the
interrupt — and `ALHARD.MAC:56` strobes it once, relying on both. Space Duel
keeps them apart.

**The cadence is the reverse of Space Duel's.** Nine interrupts per frame
(`ALEXEC.MAC:49-52` holds the mainline until `FRTIMR` reaches 9) at 250 Hz,
the 3 kHz timer over twelve: 6,048 cycles per interrupt, 54,432 per frame.
`ALWELG.MAC:580` says the game runs "28 PER SECOND" and 9 x 250 Hz is 27.8,
which is what that comment rounds. Worth recording because `sd.rs` lists the
divide-by-twelve hypothesis as *dead* for Space Duel, whose measured period is
6,144.

`te.rs` and `mbox.rs` carry 99 source citations and 12 explicit `UNVERIFIED`
markers between them; `vg.rs` gained more of both.

### Two traps worth recording

- **Tempest's display lists never halt.** `VGHALT` is called only by the
  self-test — `ALTEST.MAC:486`, "PLACE HALT AT END OF DISPLAY LIST" — and the
  main display path never places one. Its lists end in a `JMPL`, double
  buffered, and go round until the CPU stops them; `ALHARD.MAC:170-175`
  restarts the generator every interrupt on finding the halt bit set, which is
  why the strobe count is one per *interrupt* rather than one per frame. So
  budget exhaustion is the ordinary case here where it is a fault on Space
  Duel. One pass is the picture the phosphor shows: running to the budget drew
  76,000 segments for a picture of 365, and the pass detector stops when the
  generator revisits a *top-level* word — depth matters, because a glyph
  subroutine is legitimately called many times in one list.
- **The EAROM latches; it does not store.** `ALEARO.MAC:215-224` selects the
  cell to read with `STA X,EADAL`, where the address carries the cell number
  and the accumulator happens to hold the control byte. Storing on that write
  means every read destroys the cell it is reading — and the game caught it:
  `EARCND` counted 7 until the store was moved to the write-mode strobe on the
  control port. That is the whole argument for running the diagnostic.

### What attract mode does that Space Duel's does not

Six frames in 1,200 draw nothing, at 309-310, 386 and 930-932 — bursts of one
to three, the two clusters about 620 frames apart, which at 27.8 frames a
second is a twenty-two second cycle. That is the attract sequence changing
screens, clearing the display list and rebuilding it. `attract_te` asserts the
longest *run* of blank frames rather than the count, because a wedged machine
would show hundreds in a row and three is the game changing its mind.
