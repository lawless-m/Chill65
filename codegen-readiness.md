# Codegen readiness — open question O3

**How much of the code is HLL65F-structured versus bare branches?**

Plan §7 asks this, and plan §4 calls the answer "the single best predictor of
how the WASM build will behave". It was not measurable until the macro expander
existed, because HLL65F's structure only becomes visible on expansion.

Measured over the full program build (`CRF` + `CRP` + `CLS`, version-3 program
sources with root-tree data), attributing every emitted branch instruction to
its origin:

| Origin | Branches | Share |
|---|---|---|
| **HLL65F structured control flow** | **564** | **64.4%** |
| Other macros (`BINT`, `INC16`, `DEC16`, …) | 46 | 5.3% |
| Hand-written, at top level | 266 | 30.4% |
| **Total** | **876** | |

Reproduce with:

```
CHILL65_ANALYSE=1 cargo run -p chill65-asm -- CRF.MAC CRP.MAC CLS.MAC \
  -I <corpus>/version-3 -I <corpus>
```

## Method

A branch is attributed by its macro expansion chain at the moment of emission.
HLL65F emits all of its branches from inside `IFXX`, `FND`, `ELSE` or `..END`,
so a chain touching any of those is structured control flow. A branch emitted
with an empty chain was written by hand in the source.

This counts *branch instructions*, not routines, which is the right unit: it is
branches that a relooper has to make sense of, and a routine mixing both kinds
would be miscounted by any per-routine measure.

Construct invocations, for context — 1,702 in total, the most used being:

| Construct | Uses |
|---|---|
| `ENDIF` / `THEN` | 466 (the same construct; `ENDIF` expands to `THEN`) |
| `IFEQ` | 198 |
| `IFNE` | 121 |
| `ELSE` | 108 |
| `BEGIN` | 98 |
| `IFCS` | 60 |
| `EQEND` | 54 |
| `IFCC` | 40 |
| `IFMI` | 26 |

466 conditionals and 98 loops, against 266 hand-written branches.

## What this means for plan §4

Plan §4 ranks four lowering strategies. The measurement bears on the first two.

**Bucket 1 — carry HLL65F structure through.** Expected to "cover the bulk of
the code". It covers **64.4%** of branches: real, and the largest share, but
distinctly less than "the bulk". Two thirds is a good position, not a solved
problem.

**Bucket 2 — relooper fallback.** The plan treats this as the exception. On this
measurement it must handle **266 hand-written branches, 30.4% of the total** —
not a residue but a third of the control flow. The relooper is core
infrastructure, not a fallback, and should be planned and resourced as such.

**Buckets 3 and 4 are already known to be small.** `smc-gate.md` §4 established
that the entire game contains five indirect transfers, all through named tables
with bounds checks or masks, which lower to a `match` rather than
`call_indirect`. Nothing has been found that needs the interpreter fallback.

A caveat on generality: this figure is for Crystal Castles, which is one of the
ten titles in the archive that ship an HLL65F-family package. `inventory.md` §5
found the other nine — Centipede, Asteroids Deluxe, Battlezone, Red Baron and
the rest — have no such package at all. For those the structured share is zero
by construction and the relooper is the *only* mechanism. The 64.4% here is
therefore an upper bound across the archive, not a typical value.

## Caveats on the measurement

- Taken from a build that is 93.10% byte-identical to the oracle (`gate1.md`).
  The differences are confined to the sound package, so the branch attribution
  for `CRF` — which is byte-identical and holds most of the game logic — is
  reliable; `CRP`'s contribution carries the same uncertainty as its bytes.
- Branch *instructions* are counted, not basic blocks or irreducible regions.
  Whether the hand-written 30.4% is mostly reducible is a separate and more
  important question for the WASM target, and is not answered here. It should
  be, before Phase 5 is planned.
