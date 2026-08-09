# Chill65

A static recompiler toolchain for Atari 6502 arcade games, translating the
original assembly source into Rust for native and WebAssembly targets.

First target: **Crystal Castles** (Atari, 1983).

## Status

**Phase 0 and Phase 1 complete.** The front end reassembles the original Atari
source to output byte-identical with the original toolchain's own — 24,576 bytes
of program and 16,384 of castle data, both exact.

```
CHILL65_CORPUS=/path/to/crystal-castles \
  cargo test -p chill65-asm --test gate -- --ignored --nocapture
```

Phase 2 (runtime and interpreter) not started. See `atari-recompiler-plan.md`
for the full plan, and:

| Document | What it establishes |
|---|---|
| `smc-gate.md` | No self-modifying code, no computed control transfers (plan Phase 0.1) |
| `integrity.md` | Which source version to build, verified against 13 documented ROM checksums (Phase 0.2) |
| `inventory.md` | The archive: ~20 Atari coin-op titles, in two dialect generations (Phase 0.3) |
| `data.md` | Castle/wave data format (Phase 0.4) |
| `hardware.md` | Memory map, ROM banking, the OUT0 latch |
| `frontend.md` | Why the front end is written in Rust rather than reusing AT6502 |
| `gate1.md` | The Phase 1 gate: both images byte-identical, and the causes that closed the last 1,696 bytes |
| `dialect.md` | The assembler dialect as implemented, and what is deliberately not |
| `codegen-readiness.md` | Open question O3 measured: 64.4% structured control flow |
| `LOOP.md` | Working scope and stop conditions |

## You must supply your own game source

This repository contains **no game code and no ROMs**, and never will. The
`historicalsource` material is archived donated material, not a licensed
release, and Atari is an active rights holder. The toolchain, runtime and
hardware models here are original work; emitted game code is not distributable.

This is the same posture the MiSTer FPGA core takes — ship the implementation,
require the user to provide the game.

To work on the toolchain you will want local clones of the reference material.
All are excluded by `.gitignore`:

```
git clone https://github.com/historicalsource/crystal-castles
git clone https://github.com/historicalsource/atari-coin-op-assembler
git clone https://github.com/MiSTer-devel/Arcade-CrystalCastles_MiSTer
```

## Layout

```
crates/chill65-asm/   assembler front end for the Atari MACRO-11 dialect
tools/                Python analysis scripts (LDA decoder, source scanners)
```

## Build

```
cargo check
cargo test
```

No third-party crate dependencies.
