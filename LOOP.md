# Loop scope and end condition

Driving: `/loop /task-loop` in dynamic mode. One task per slice, 60s cadence.

## Campaign 3 (current): Phase 2 — runtime and interpreter

Tasks **#18–#31**. Build a headless 6502 runtime that runs Crystal Castles
interpreted, from ROM images our own assembler produces.

Shape: scaffold (#18) → CPU in three slices (#19–#21) → Klaus functional-test
validation (#22) → bus and banking (#23) → video, timing, POKEY, input
(#24–#27) → ROM load and boot (#28) → self-test gate (#29) and headless runner
(#30) → **#31 the terminal gate**.

**Testable end condition** — five commands, all exit 0 in one run:

```
cargo build
cargo test
CHILL65_CORPUS=<corpus> cargo test -p chill65-asm --test gate -- --ignored
CHILL65_CORPUS=<corpus> cargo test -p chill65-runtime -- --ignored
CHILL65_KLAUS=<clone>   cargo test -p chill65-runtime --test klaus -- --ignored
```

The third is the Phase 1 byte-identical gate, included so Phase 2 cannot
regress Phase 1 unnoticed.

**Why the self-test is the gate rather than "playable".** The plan's stated
Phase 2 gate is "game runs, fully interpreted, playable" — a human judgement,
useless to an unattended loop. But the game carries its own diagnostic:
`CST.MAC:312 ROMTST` / `:352 GOTCHK` checksums all five 2764s. Passing it means
CPU, bus, banking and timing are correct enough for Atari's own test to agree,
and because the five ROMs span *both* banks it exercises bank-sensitive reads end
to end. "Playable" remains the later human milestone; #30's PPM dump is the
artefact for judging it.

**Deliberately out of scope**, for Matt to decide separately: windowing, audio
output, and any third-party crate for either. The runtime core stays
dependency-free and headless.

## Campaign 2 (closed): close the gate

The first campaign built the front end and got the data image byte-identical
with the program at 93.10% (`gate1.md`). The current list is the remainder:

- **#15** diagnose the CRP.MAC divergence (measurements, not conjecture)
- **#16** eliminate the 16 large content runs (blocked by #15)
- **#17** drive the program image to zero differing bytes (blocked by #16)
- **#13** flip `tests/gate.rs` to strict equality and close Phase 1 (blocked by #17)

**Testable end condition**, the whole point of the list:

```
CHILL65_CORPUS=<corpus> cargo test -p chill65-asm --test gate -- --ignored
```

passes with `gate.rs` asserting **zero differing bytes on both images**. Until
#13 flips it, `gate.rs` asserts two-sided ratchet bounds instead, so progress
can only move one way.

## Terminus

**Task #13 — the Phase 1 byte-identical gate — is the end of this loop.**

When #13 completes, the list drains and the loop stops. It does not roll on into
Phase 2. The interpreter and runtime are a materially larger commitment than
everything before them and deserve a fresh decision with Phase 1's real costs
known, rather than an automatic continuation.

The gate is self-contained: the oracle is `CRF.LDA` / `C99.LDA`, assembled output
of the original toolchain already present in the source tree. No MAME ROM set is
needed, nothing third-party has to be obtained, and nothing game-derived enters
this repository (plan §9).

## Multi-slice tasks are normal — do not halt for them

A task that outlasts one slice is **in progress**, not failed. Keep it
`in_progress`, schedule the next wake-up as usual, and resume it at the top of
the next slice.

This needs stating because `task-loop` selects from `status: pending`, so an
`in_progress` task is invisible to its filter. On resuming, **check for an
in-progress task first and continue that** before applying the lowest-ID-pending
rule. Otherwise the loop reports "list drained, loop complete" while real work
remains — which is worse than a spurious halt, because it looks like success.

Halting is for the conditions below only. Being large is not one of them.

## Halt conditions — stop and report, do not schedule

Beyond the skill's own rules (task failed, verification failed, description
ambiguous):

1. **Task #5 recommends reusing AT6502.** Tasks #6–#13 are built on the Rust
   assumption. That decision is Matt's, not the loop's.
2. **Task #2 cannot identify a buildable tree.** Everything downstream needs a
   chosen source version.
3. ~~**Task #11 cannot determine the `AY,` addressing mode.**~~ **RESOLVED by
   task #5.** Atari's own opcode processor
   (`atari-coin-op-assembler/atari_tools/e2_tools/OPC65.MAC`) enumerates all
   fourteen addressing modes; `AY` is absolute,Y. No guessing required, and this
   halt condition no longer applies.
4. **Task #13 finishes with unexplained byte differences.** Characterised
   differences written up in `gate1.md` are a halt, not a completion.
5. **GitHub rate-limiting during task #4.** Record how far the survey reached and
   stop; do not sleep-and-retry inside a slice.
6. **Any task fails verification on two separate slices.** Two failures on the
   same task means the task is wrong, not the attempt. Stop and let Matt look.

## Revisions to the list

The task list is refined as evidence arrives; changes are recorded here rather
than made silently.

- **#12 widened** (from task #2): the build is four assemblies and two links, not
  two assemblies, so a LINKM-equivalent linker is in Phase 1 scope.
- **#11 sharpened** (from task #5): the `AY,` addressing mode is resolved, and
  the full fourteen-mode set comes from Atari's own `OPC65.MAC`.
- **#12 re-scoped — no linker** (Matt's call): the original build's four
  assemblies and two link steps were a tooling constraint of 1977–83, not a
  semantic requirement. We are unconstrained, so we implement the scoping the
  source already declares (`NAME:` private, `NAME::` global, `=` vs `==`,
  `.GLOBL`) and assemble all four roots in one run against one image. This
  changes *mechanism only* — emitted bytes must still match exactly, including
  the `?` XOR checksums and `LDAL`'s byte punning.

  **Correction to the evidence given for this.** I originally reported three
  shared names across the modules (`$INTCT`, `ATRACT`, `START`) as deliberately
  separate variables that a naive merge would corrupt. That overstated it: all
  three of CRP's definitions are inside `.IF NE,...TST`, and `CRP.MAC:45` sets
  `...TST = 0` ("SELF-TEST MODE... 0 = OPERATIONAL MODE"). In an operational
  build there are **zero** private-symbol collisions and a shared table would
  have worked. Scoping remains the right design — it is the rule the source
  declares, it costs nothing, and a self-test build needs it — but it was not
  load-bearing in the way I claimed. Macro scoping *is* load-bearing: `LDAL`
  and `LDAH` are genuinely defined twice, unconditionally, with different
  bodies.
- **#10 split** (mid-slice): it bundled the macro expansion core with assembler
  integration *and* the whole HLL65F structured-control-flow package — two
  slices of work behind one VERIFY line. The core became #10 (done); the rest is
  **#14**, deliberately sequenced *after* #11 so `LOC`/`FND` back-patching has
  real instruction sizes to measure instead of a stub seam. The loop halted
  rather than mark #10 done with four of seven tests missing.

## Anti-stall notes

- Every task carries an explicit `VERIFY:` line. A task with no runnable
  verification is a task that can be marked done on a feeling — none were written
  that way.
- Tasks #3 and #4 are unblocked and independent of the Phase 1 chain, so the loop
  has work available even if the front-end chain halts.
- The `.LDA` oracle removes the one dependency that could have blocked the whole
  list indefinitely (acquiring ROMs).
- Tasks #7–#11 each end in `cargo test`, so a slice cannot report success on code
  that does not compile.

## Not in scope for this loop

- Phase 2 runtime, interpreter, video model, POKEY, trackball.
- Phase 3 MAME differential harness.
- Phases 4–6: emitter, WASM, second game.
- The level viewer floated in open question O5.
- The six amendments to `atari-recompiler-plan.md` raised in review — still parked
  awaiting Matt's decision; the plan document remains unedited.
