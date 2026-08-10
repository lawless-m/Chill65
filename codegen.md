# Lowering 6502 to Rust

Phase 4's deliverable in progress: what the emitter does, what resisted it, and
what the numbers are. Figures here are measured by commands that exist, named
beside each.

## 1. Migration wave 1

Seven routines compiled, each added to `crates/chill65-native/routines.txt`
alone and then proved on all five committed traces before the next was added.

```
CHILL65_CORPUS=<corpus> cargo run -q -p chill65-native --bin ccnative -- \
  verify --trace traces/<t>.trace
```

| Routine | Entry | Instructions lowered |
|---|---|---|
| `RS.INI` | `A22B` | 98 |
| `WV.BDR` | `AA33` | 60 |
| `ROMTS1` | `ECE7` | 25 |
| `LN.F1` | `A407` | 20 |
| `LN.F2` | `A462` | 23 |
| `SQ.LDR` | `AC8C` | 36 |
| `SQ.SQD` | `AC34` | 38 |

Coverage after the wave, from `ccnative coverage` and the gated test in
`chill65-native`:

| Trace | Instructions | Compiled |
|---|---|---|
| `coin-start.trace` | 5,559,710 | 2.5% |
| `gameplay.trace` | 15,844,630 | 1.3% |
| `idle-attract.trace` | 3,950,048 | 3.5% |
| `selftest.trace` | 3,677,393 | 0.0% |
| `wrap-stress.trace` | 11,491,747 | 1.2% |
| **aggregate** | **40,523,528** | **1.5%** |

1.5% is not a disappointment, it is the measurement the plan asked for. See §3.

## 2. What refused, and why

Asked for the twelve hottest routines, the emitter took seven. The five it
refused are more informative than the seven it took:

| Routine | Reason |
|---|---|
| `MN.ST` | hand-written branch at `EBB3` (`BNE`) — needs the relooper |
| `MN.FRA` | hand-written branch at `EA5D` (`BCC`) — needs the relooper |
| `CL.TRA` | hand-written branch at `A7FB` (`BEQ`) — needs the relooper |
| `BITEST` | not a routine in the IR |
| `STJUMP` | not a routine in the IR |

`BITEST` and `STJUMP` are labels *inside* routines rather than routines
themselves — the coverage report names them because it resolves a program
counter to its nearest preceding symbol, which is the right thing for a profile
and the wrong thing for a unit of compilation. They are not missing; they are
part of whatever routine contains them.

## 3. The hot code is the relooper's

The three refused for hand-written branches are the ones that matter.
On `idle-attract.trace`:

| Routine | Executed | Share |
|---|---|---|
| `MN.ST` | 1,384,124 | 35% |
| `BITEST` | 769,695 | 19% |
| `MN.FRA` | 435,851 | 11% |

**`MN.ST` alone is a third of everything the machine executes**, and it is
refused. That is the whole reason 1.5% is where bucket 1 lands: HLL65F covers
64.4% of *branches* (`codegen-readiness.md`), but the code that runs hottest is
disproportionately hand-written.

`codegen-readiness.md` reached the same conclusion statically and said so:
*"The relooper is core infrastructure, not a fallback, and should be planned and
resourced as such."* This is that prediction confirmed dynamically. Bucket 1 was
never going to reach a majority on its own.

There is a second reason to care. `inventory.md` §5 found that of the ten
archive titles with an HLL65F-family package, Crystal Castles is one — the other
nine have none at all, so their structured share is zero by construction. The
relooper is not merely how this game reaches a majority; it is the only front
end the rest of the archive will ever have.

## 4. Three bugs the differential runs caught

Each was found by running, not by reading, and each is the kind of thing that
looks perfectly reasonable in the generated source.

**Loops had the wrong polarity.** `xxEND` names the condition that *ends* the
loop and emits the **inverse** branch back to the top — `dialect.md` records
`BEGIN … PLEND` assembling to `ea 30 fd`, a `BMI` backwards. The emitter had it
as "loop while the condition holds". `ROMTS1` is the ROM checksum, so a
mis-lowered loop computed a different checksum, the self-test reported a
different result, and the failure surfaced as `BITEST` drawing 531 different
pixels at frame 335 — three steps removed from the cause.

Worse, the fixture had been written to match the *implementation* rather than
the dialect, so it passed. Only the real game caught it. A fixture that encodes
the same misreading as the code it tests is worse than no fixture, because it
buys confidence it has not earned.

**`ELSE` emits a jump nobody was charging.** `HLL65F.MAC`'s `ELSE` is `JMP .`,
its target back-patched by `FND` — an unconditional jump over the else-block.
It appears in the event stream *after* the `ELSE` marker, so the emitter lowered
it as the first instruction of the else-branch: a yield-and-return that skipped
the else-block entirely. `SQ.SQD` is the only wave-1 routine using `ELSE`, and
it diverged by five pixels at frame 371. The jump belongs to the *end of the if
branch*, and is now charged there with its jump replaced by the Rust `else`.
`ELSE` is used 108 times in the game, so this would have blocked most of wave 2.

**Closing markers fell off the end of routines.** A marker is recorded at the
address of the next instruction, so a construct closing at a routine's end lands
on the *following* routine's entry and was dropped, leaving the routine reading
as unbalanced. That is what refused `MN.ST` and `MN.FRA` at first — with a
misleading reason. Boundary attribution now assigns openers on the entry address
and closers on the end address to the routine they belong to, and each marker
lands in exactly one.

## 5. How a migrated routine is proved

`ccnative verify` plays a trace with dispatch off and on and requires identical
per-frame hash streams and cycle counts. On divergence it localises exactly as
Phase 3 does: both sides are ours, both carry write logs, so blame merges from
both and the report names the frame and the routines. Nothing new was invented
for checking compiled code — the harness built to compare against MAME compares
against ourselves just as well.

## 6. Still to come

The relooper (bucket 2), migration wave 2, and the majority gate. Sections on
the lowering contract, the buckets' final ratios, and what resisted lowering
routine by routine will be completed when those land.
