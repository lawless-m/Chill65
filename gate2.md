# Phase 2 gate — PASSED

The runtime runs Crystal Castles from source assembled by our own toolchain, and
the game's own diagnostic agrees the bytes are right.

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

## Next

Phase 3 — the MAME differential harness. The plan is emphatic that it be built
**before** the emitter. That is a fresh decision, not a continuation.
