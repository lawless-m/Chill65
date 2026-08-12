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
