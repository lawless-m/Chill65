# Phase 4 gate — PASSED

A majority of the code the machine actually executes is compiled Rust, and it is
indistinguishable from interpreting it.

## The seven commands, one run, all exit 0

| # | Command | Result |
|---|---|---|
| 1 | `cargo build` | clean |
| 2 | `cargo test` | **287 passed**, 0 failed, 29 ignored |
| 3 | `cargo test -p chill65-asm --test gate -- --ignored` | **byte-identical, both images** |
| 4 | `cargo test -p chill65-runtime -- --ignored` | 6 passed |
| 5 | `cargo test -p chill65-runtime --test klaus -- --ignored` | 2 passed |
| 6 | `cargo test -p chill65-diff -- --ignored` | 8 passed, both oracles live |
| 7 | `cargo test -p chill65-native -- --ignored` | **66.0% compiled**, majority asserted |

### 3, 4 and 5 — Phases 1 and 2 unregressed

```text
PROGRAM: 24576 of 24576 bytes match (100.00%), 0 differ
DATA:    16384 of 16384 bytes match (100.00%), 0 differ

attract: 600 frames, 12288528 cycles, hash 087e02f4ca003874
booted:  420 frames, 8601893 cycles, 200 interrupts taken
verdict  Passed { checksums: [1, 2, 3, 4, 5] }
corrupted ROM correctly rejected: checksums [00, 02, 03, 04, 05]
Klaus:   decimal and functional suites both pass
```

Every figure identical to `gate2.md` and `gate3.md`. Phase 4 added an
instruction dispatch to the frame loop, made two dozen opcode helpers public,
and changed colour RAM's reset state — and moved none of them, because
`frame_hash` hashes palette indices rather than colours.

### 6 — Phase 3 unregressed, both oracles exercised

```text
clean vs clean:  identical over 600 frames
clean vs faulty: diverged at frame 419: LN.F1 [CRF.MAC] (85 pixels), …
mame determinism:   500 frames identical across two runs
mister determinism:  60 frames identical across two runs
ours vs mame:    diverged at frame 162: MN.ST [CRF.MAC] (2 pixels)
ours vs mister:  diverged at frame 162: MN.ST [CRF.MAC] (2 pixels)
ROM set:         11 of 11 devices match MAME's CRC-32
```

The harness still localises an injected fault to its own routine, and both
external oracles still agree with each other about the two-pixel difference at
frame 162.

### 7 — the majority

```text
coin-start.trace     900 frames    5,559,710 instructions   76.4% compiled
gameplay.trace      2640 frames   15,844,630 instructions   49.8% compiled
idle-attract.trace   600 frames    3,950,048 instructions   69.9% compiled
selftest.trace       600 frames    3,677,393 instructions   57.8% compiled
wrap-stress.trace   2006 frames   11,491,747 instructions   84.4% compiled

aggregate: 26,734,538 of 40,523,528 executed instructions compiled (66.0%)
```

## What this gate asserts

Two things, per trace and in aggregate:

- **Identity.** Every trace produces the same per-frame picture hashes and the
  same cycle count whether routines are dispatched or interpreted. That is what
  makes a migration safe rather than merely fast, and it is checked on all five
  committed traces, not a sample.
- **Majority.** Compiled instructions over total executed instructions exceeds
  50%.

**Executed instructions** is the unit, deliberately. Counting routines would let
a hundred cold ones be migrated and a majority declared; counting bytes would do
the same for a large table-driven routine that never runs. Only execution counts
weight a routine by how much it actually matters.

The assertion was written when the manifest was empty and the meter read 0.0%,
so it has been seen reading zero as well as reading a majority.

## What it deliberately does not assert

That compiled output is *faster*. Nothing here measures wall clock. Phase 4's
claim is that the translation is faithful and that a majority of execution has
moved; performance is Phase 5's business, and the state-machine lowering (below)
is the wrong shape for it.

Nor does it assert agreement with MAME or the MiSTer core — that remains Phase
3's output rather than a pass condition, for the reasons `gate3.md` gives.

## The three bugs the differential runs found

None would have been found by reading the generated code, and each looked
entirely reasonable on the page.

**Loop polarity.** `xxEND` names the condition that *ends* a loop and emits the
**inverse** branch back to the top — `dialect.md` records `BEGIN … PLEND`
assembling to `ea 30 fd`, a `BMI` backwards. `ROMTS1` is the ROM checksum, so a
mis-lowered loop computed a different checksum, the self-test reported a
different result, and it surfaced as `BITEST` drawing 531 different pixels at
frame 335 — three steps from the cause.

The fixture had been written to match the implementation rather than the
dialect, so it passed. Only the real game caught it. **A fixture that encodes
the same misreading as the code it tests is worse than no fixture**, because it
buys confidence it has not earned.

**`ELSE`'s uncharged jump.** `HLL65F.MAC`'s `ELSE` is `JMP .`, back-patched by
`FND` — a jump over the else-block that appears *after* the marker. It was
lowered as the first instruction of the else branch: a yield-and-return that
skipped that branch entirely. Five pixels at frame 371. `ELSE` is used 108 times
in the game.

**Markers falling off routine ends.** A closing construct records at the address
of the next instruction, so one ending a routine landed on the following
routine's entry and was dropped. That refused `MN.ST` and `MN.FRA` at first,
with a misleading reason.

## The finding that mattered most

With twenty routines compiled the aggregate sat at **34.5%**, and the reason was
invisible until the coverage report counted compiled and interpreted separately
instead of stamping each routine with one status:

```text
executed  compiled  interpreted  routine
 1384124        20      1384104  MN.ST
  769695         4       769691  RAMOK
```

`MN.ST` was dispatched, ran twenty instructions, hit a `JSR`, yielded — and the
remaining 1.38 million interpreted, because dispatch fired only at a routine's
*entry*. The report had called that routine "compiled", which was true and
useless.

Making state machines resumable at any block took the aggregate from 34.5% to
**66.0% with no new routines migrated**. The same twenty, re-entered.

## Unresolved, named rather than glossed

- **Reducible flow is not recovered into `loop`/`if`.** Bare branches become a
  block-dispatch state machine whatever their shape. Plan §4 ranks structure
  recovery above the fallback and for **WASM it matters** — a `loop`/`match` is
  exactly what LLVM cannot optimise across, and §4 exists because WASM permits
  no arbitrary jumps. This is the largest piece of work Phase 5 inherits.
- **Structured routines are single-entry**, since Rust has no computed entry
  point. One that yields mid-way still interprets its tail, which is part of why
  `gameplay.trace` is lowest at 49.8%.
- **The two-pixel `MN.ST` divergence at frame 162** is still unexplained, and
  `MN.ST` is now compiled — so the difference is in our model, not our lowering,
  and both external oracles agree on it.
- **`EN.STC` was refused** and not investigated; the majority was reached
  without it.
- The remaining interpreted 34% is **unattempted, not resistant**. The manifest
  is a short list of hot routines rather than a sweep.

## For Phase 5

The WASM target inherits a working creep line and a lowering that is the wrong
shape for it. `codegen-readiness.md`'s closing caveat is the thing to measure
first: whether the hand-written 30.4% is mostly *reducible*, which it calls "a
separate and more important question for the WASM target" and does not answer.
Now that those routines are compiled and verified, that question can be asked of
real generated code rather than of source text.

## Distribution

Unchanged and unmoved by any of this. Emitted Rust is generated from Atari's
source and is as undistributable as the ROM (plan §9): produced into `OUT_DIR`
at build time, never committed. What is committed is `routines.txt` — routine
names, our own reading of the source — and an original fixture program
containing no game code, which is why `cargo test` exercises the entire pipeline
on a machine with no corpus at all.
