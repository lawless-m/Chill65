# Phase 3 gate — PASSED

The differential harness works: it drives our runtime and two independent
external implementations from identical recorded input, and when a difference is
introduced in a known place it names the right frame and the right routine,
reproducibly.

## What this gate asserts, and what it deliberately does not

It does **not** assert that our runtime agrees with MAME or with the MiSTer
core. `gate2.md` records six claims about the hardware still marked UNVERIFIED,
and motion objects are not modelled at all, so divergence against an external
oracle is the harness's *output* rather than its failure condition. The plan is
explicit (`atari-recompiler-plan.md:129`): *"Purpose is debugging, not
certification: the useful output is 'diverged at frame 4,812, routine X'."*

A gate demanding frame equality would be one the project cannot pass, and an
unattended loop would grind against it forever.

So the gate is that the **machinery** works. The precedent is
`chill65-runtime/tests/selftest.rs`, which proves the ROM self-test *fails* on a
corrupted byte rather than merely that it passes: a diagnostic you have never
seen fail is not evidence of anything.

## The six commands, one run, all exit 0

| # | Command | Result |
|---|---|---|
| 1 | `cargo build` | clean |
| 2 | `cargo test` | **262 passed**, 0 failed, 23 ignored |
| 3 | `cargo test -p chill65-asm --test gate -- --ignored` | **byte-identical, both images** |
| 4 | `cargo test -p chill65-runtime -- --ignored` | 6 passed |
| 5 | `cargo test -p chill65-runtime --test klaus -- --ignored` | 2 passed |
| 6 | `cargo test -p chill65-diff -- --ignored` | 8 passed, both oracles live |

### 3 and 4 — Phases 1 and 2 unregressed

```text
PROGRAM: 24576 of 24576 bytes match (100.00%), 0 differ
DATA:    16384 of 16384 bytes match (100.00%), 0 differ

attract: 600 frames, 12288528 cycles, hash 087e02f4ca003874
booted:  420 frames, 8601893 cycles, 200 interrupts taken
verdict  Passed { checksums: [1, 2, 3, 4, 5] }
corrupted ROM correctly rejected: checksums [00, 02, 03, 04, 05]
```

The attract hash and cycle count are identical to the figures `gate2.md`
recorded, which is the evidence that Phase 3's runtime instrumentation changed
nothing. It is a measurement, not a hope.

### 6 — the harness

```text
ROM set:      11 of 11 devices match MAME's CRC-32
traces:       5 files play deterministically, none expire the watchdog
              selftest 354, coin-start 403, gameplay 403, wrap-stress 403
clean vs clean:  identical over 600 frames
clean vs faulty: diverged at frame 419: LN.F1 [CRF.MAC] (85 pixels),
                 SQ.LDR [CRF.MAC] (50 pixels), WV.BDR [CRF.MAC] (35 pixels)
mame:         boots the rebuilt set, 15,283 lit pixels by frame 399
              deterministic over 500 frames, input lands at frame 402
mister:       verilated unmodified, 252 emitted columns, deterministic
ours vs mame:   diverged at frame 0: 59392 pixels differ, none attributable
ours vs mister: diverged at frame 0: 59392 pixels differ, none attributable
```

## The fault-injection fixture, and why it is a pair of flips

The obvious fixture does not test what it appears to. The game checksums its own
ROMs at power-on, so *any* altered program byte is caught, and every such fault
produces the same picture — the self-test's failure screen. Measured, that is a
divergence at frame 371 with 56,836 of 59,392 pixels differing, all attributed
to `BITEST`. The flipped byte's own routine never gets a chance to draw anything
wrong. Two entirely different bit flips gave byte-identical reports, which is
what gave it away.

The checksum is a longitudinal parity — `EOR NY,TEMP1`, `CST.MAC:331` — so
flipping the same bit in **two** bytes of one 8K device leaves it unchanged. The
fixture flips bit 0 at `A408` and `A418`, both inside `LN.F1`, both in
`136022-303.1k`:

- `A418` is the `32` of `LDA $32`, the zero-page byte holding the X coordinate.
  Flipped, the routine reads `$33` — the Y coordinate — and draws in the wrong
  place. This is the fault.
- `A408` is the `00` of `LDA #$00`, whose value reaches only the OUT1 latch.
  That latch samples **D3 only** (`CCastles.v:332`), and bit 3 of `00` and `01`
  agree, so this flip changes nothing observable. It exists to restore the
  parity.

One flip does the damage, the other is invisible, and together they satisfy the
game's own diagnostic. The harness then finds the fault at frame 419 and blames
`LN.F1` — the routine holding the flipped byte — without being told, twice
running.

## Which oracles were exercised

Both. MAME 0.276 and Verilator 5.032 are installed on the development machine
and command 6 was run with `CHILL65_MAME` set, so no oracle test skipped.

Every oracle test skips cleanly when its tool is absent, and clean skips do not
fail the gate — but this run did not need that leniency.

## What Phase 3 established

- A recorded input format, and five committed traces whose timings come from the
  game's own source rather than guesswork.
- A pluggable oracle seam, with three implementations behind it: our runtime,
  MAME, and a verilated MiSTer core.
- Routine attribution from a per-pixel last-writer log, keyed by RAM address so
  that scrolling cannot silently corrupt it.
- MAME's `ccastles3` ROM set rebuilt from the game source alone, all eleven
  devices matching MAME's own CRC-32 — verified by `mame -verifyroms` on an
  archive our own ZIP writer produced. No ROM was downloaded to make it.
- Two independent confirmations of the input model: MAME's `:IN0` masks land on
  exactly the bits `Switch::bit` names, and its `LETA0`/`LETA1` are vertical then
  horizontal as `input.rs` says.

## What it found, and what it could not

The harness's first real finding is a disagreement about **colour RAM at
power-on**:

| Implementation | Frame 0 |
|---|---|
| MAME | black |
| MiSTer core | black |
| `chill65-runtime` | **white** |

`Video::new` zeroes all 32 CRAM entries, and `cram_rgb` inverts every component,
so a zeroed entry 16 — the one a bitmap pixel of zero selects — decodes to full
white. The MiSTer core's `cram.rom` sets exactly that entry to `1FF`, which
inverts to black, and MAME starts black too.

On real hardware colour RAM is RAM and its power-on contents are undefined, so
this is not a proven modelling error. But it has a concrete consequence: **both
external comparisons diverge at frame 0 for a reason unrelated to drawing, and
that divergence masks everything after it.** Both reports are accurate and
useless in equal measure.

Two remedies, either of which would make the external comparisons informative:
initialise entry 16 to `1FF` to match both oracles, or compare from a frame
after the game has written CRAM. Neither is taken here. This is a decision about
what the harness measures and belongs to a human.

Also unresolved, and named rather than glossed:

- `coin-start.trace` parts from idle at MAME frame **402** against our **403**.
- The MiSTer core's picture arrives roughly **200 frames later** than ours: 176
  lit pixels by frame 350 and 32,573 by 450, against our first draw at 162.
- The core emits **252 of 256 columns** — `HBLANK2` blanks `RGBout` over the
  rest — so those columns cannot be observed at its ports at all.
- Which line of each trackball quadrature pair leads is not established; a
  reversed axis would show as inverted motion.

## For Phase 4

The emitter migrates routines from interpreter to compiled Rust one at a time,
re-diffing after each. What that needs from here:

- `localise` for clean-versus-migrated comparison — both sides ours, both with
  write logs, so blame merges from both. That is the fault-injection path
  already gated, with a migrated routine in place of a flipped bit.
- `traces/` as the regression corpus: any migration that changes a trace's hash
  stream has changed behaviour, and the harness will say in which routine.
- The frame-0 CRAM question settled first if the external oracles are to be
  useful during migration, since until then they report only that one fact.

## Deliberately out of scope

Windowing and audio output, still. The core stays headless and the workspace
still has **zero third-party crate dependencies** — CRC-32, the ZIP writer, the
`.LDA` decoder and every argument parser here are hand-rolled.
