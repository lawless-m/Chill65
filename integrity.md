# Phase 0.2 — source version selection and integrity

**Decision: build the program from `version-3/`, and the castle data from the
repository root.** Reasoning in §6. Every claim below cites the file it came from.

Subject: `github.com/historicalsource/crystal-castles` @ `9e9673a`.

---

## 1. The three trees identify themselves

Each tree carries an Atari release document. They are unambiguous.

| | Root | `version-2/` | `version-3/` |
|---|---|---|---|
| Document | `372X1.DOC` | `version-2/372X2.DOC` | `version-3/372X3.DOC` |
| Version | 1 | 2 | 3 |
| Date | 6-20-83 | 6-23-83 | 7-11-83 |
| Project name | **TOPOROIDS** | CRYSTAL CASTLES (TOPOROIDS) | CRYSTAL CASTLES |
| Programmer | Franz Lanzinger | Franz Lanzinger | Franz Lanzinger |
| Project leader | *(blank)* | *(blank)* | Scott Fuller |
| Doc disks | 59A & 59B | 59D, 59E, 59F | 59G–59I |
| Verification files | `C99.LDA`, `CRF.LDA`, `372BR.RS4` | same | **`CRF.LDA` only** |
| Program ROMs | 136022-**103/104/105** | 136022-**203/204/205** | 136022-**303/304/305** |

The original working title was *Toporoids*, which explains the `TP*` prefixes on
the PROM files (`TPSYNC`, `TPBUS`, `TPI`) and `TOPOWP`.

All three name the same toolchain: **assembler MAC65, linker LINKM**.

### Correspondence to MAME sets

Program ROM part numbers map directly onto MAME's Crystal Castles sets, which are
distinguished by exactly these numbers:

- Root (v1) → `136022-103/104/105` → **`ccastles1`**
- `version-2` → `136022-203/204/205` → `ccastles2`
- `version-3` → `136022-303/304/305` → `ccastles3` — **and the set the MiSTer core
  uses** (`ProgramMemory.v` names `136022-303.1k`, `-304.1l`, `-305.1n`, plus
  `-101.1f` and `-102.1h` for the data bank).

## 2. ROM checksums — 13 of 13 verified

The documents state a checksum per ROM. Extracting the `.LDA` images with
`tools/lda.py` and taking a plain 16-bit sum of each 8K region reproduces **every
one of them**:

| Tree | Part | Loc | Range | Documented | Computed |
|---|---|---|---|---|---|
| root | 136022-103 | 1K | A000–BFFF | 2710 | 2710 |
| root | 136022-104 | 1L | C000–DFFF | 9179 | 9179 |
| root | 136022-105 | 1N | E000–FFFF | A1C8 | A1C8 |
| root | 136022-102 | 1H | A000–BFFF | 90E9 | 90E9 |
| root | 136022-101 | 1F | C000–DFFF | A066 | A066 |
| v2 | 136022-203 | 1K | A000–BFFF | 5798 | 5798 |
| v2 | 136022-204 | 1L | C000–DFFF | AAFF | AAFF |
| v2 | 136022-205 | 1N | E000–FFFF | 554E | 554E |
| v2 | 136022-102 / -101 | 1H / 1F | — | 90E9 / A066 | 90E9 / A066 |
| v3 | 136022-303 | 1K | A000–BFFF | 5AF6 | 5AF6 |
| v3 | 136022-304 | 1L | C000–DFFF | CF7F | CF7F |
| v3 | 136022-305 | 1N | E000–FFFF | 41AC | 41AC |
| root | 136022-106 / -107 | 8D / 8B | 372BR.RS4 | 7122 / A09B | 7122 / A09B |

Three consequences:

1. The `.LDA` parser is correct.
2. **The last-write-wins resolution of the 624 conflicting record writes recorded
   in task #1 is correct** — the resolved image reproduces Atari's own documented
   checksum. That question is closed.
3. The archive's binaries are the genuine shipped ROM contents, not approximations.

`372BR.RS4` is not an `.LDA` — it is a **raw 16,384-byte binary** holding the two
motion-object (sprite) ROMs back to back, `-106` at 0 and `-107` at 0x2000.

## 3. The build is four assemblies and two links

From `372X1.DOC`, verbatim:

```
ASSEMBLY PROCEDURES  MAIN PROGRAM: A CRF
                     DATA        : A C99
                     SOUNDS      : A CRP
                     SOUND DATA  : A CLS
LINK PROCEDURES      @ L CRF,CRP,CLS      L C99
```

So `CRF.LDA` is the **linked** output of three separate assemblies — `CRF`, `CRP`
and `CLS` — and `C99.LDA` is a fourth, linked alone. `version-3/372X3.DOC` gives
the same link line (`L CRF,CRP,CLS`) and omits `C99` entirely.

This is almost certainly the source of the 624 overlapping writes: three modules
contributing records into one image.

## 4. Buildability — include closure per tree

Resolving `.INCLUDE` transitively from each build root:

| Root | Root tree | `version-2` | `version-3` |
|---|---|---|---|
| `CRF.MAC` | complete (28 files) | complete (28) | complete (28) |
| `CRP.MAC` | **MISSING `CJTB.MAC`** | **MISSING `CJTB.MAC`** | complete (2) |
| `CLS.MAC` | complete | complete | complete |
| `C99.MAC` | complete (21 files) | complete (21) | **ROOT ABSENT** |

The gaps are exactly complementary:

- **Root and version-2 cannot build the program.** `CRP.MAC` includes `CJTB.MAC`,
  which is absent. Since `CRF.LDA` is the link of `CRF`+`CRP`+`CLS`, the missing
  file blocks the whole program image, not just sound. This is the plan's §2
  "one source file was missing and reconstructed from ROMs" — now identified by
  name. `version-3/CJTB.MAC` survives: 1,346 real bytes, self-described as
  "jumptable for sounds, precedes CRP.MAC", five `JMP` entries (`ISND`, `PKDR`,
  `SNS`, `TN2`, `TN3`) followed by macro definitions.
- **version-3 cannot build the data.** `C99.MAC` and all sixteen `C**.DAT` files
  are absent, as is `372BR.RS4`.

## 5. What actually differs between trees

- Root vs version-2: 40 files identical, **14 differ** — `CEN`, `CG`, `CGR`,
  `CIN`, `CMN`, `CMS`, `CMTB`, `CRF`, `CST` (`.MAC`), plus `CRF.LDA` and the four
  PROM files. Three days between the two documents.
- Root vs version-3: only **15 files identical**, including `HLL65F.MAC`,
  `CSTART.MAC`, `CMAC.MAC`, `CLS.MAC` and — importantly — **`C99.LDA`**.
- Unique to version-3: `CJTB.MAC`, `CRM.MAC`, `CRN.MAC`, `CEEDUM.MAC`,
  `CRB.LDA`, `CRM.LDA`, `CRN.LDA`, `CLS.SND`, `CWORDS.TXT`.

**`C99.LDA` is byte-identical between root and version-3.** The castle data never
changed across revisions — consistent with the data ROMs keeping their `-101`/`-102`
part numbers in all three documents while the program ROMs advanced `1xx`→`2xx`→`3xx`.
This is what makes the split decision in §6 safe.

`version-3/CRB.LDA` is a one-byte patch writing `0x60` (`RTS`) to `E008`.
`CRM.LDA` and `CRN.LDA` are partial images with coverage gaps.

## 6. Decision

**Program: `version-3/`. Castle data: repository root.**

For the program:
1. It is the **only tree whose program source is complete** (§4). Root and
   version-2 require reconstructing `CJTB.MAC` before they can build at all.
2. Its ROM set is the one the **MiSTer core implements**, so every hardware
   question we take to the Verilog is answered about the same binary we are
   building. That corroboration path has already paid twice (`hardware.md` §3, §4).
3. It is the last revision and the only one with a project leader signed on.
4. Its `CRF.LDA` oracle verifies against all three documented checksums.

For the data: version-3 has no data source at all, root's `C99.MAC` + `C**.DAT`
closure is complete, and root's `C99.LDA` is byte-identical to version-3's — so
building data from root is not a compromise, it is the same artefact.

For the PROMs and sprites: take `372BR.RS4` from root (both checksums verify) and
the four PROM files from **version-2** (see §7).

## 7. Integrity defect — the root tree's PROM files are damaged

`TPSYNC.LDA`, `TPBUS.LDA`, `TOPOWP.LDA`, `TPI.LDA` exist in root and version-2.

- **version-2's parse cleanly and match all four documented checksums** (09E2,
  0D80, 08E8, 008C).
- **Root's do not parse at all.** Their record headers read `01 2c …` where the
  format requires `01 00 2c 00 …` — the `0x00` marker and address bytes are absent.

Cause not diagnosed. It is *not* simply version-2's file with NUL bytes stripped:
de-NUL'd lengths differ (280 vs 287, 281 vs 287, 229 vs 247, 117 vs 115), so
content differs beyond NUL removal. Nor do the root files reproduce the documented
checksums when read as raw dumps at 128, 256 or 512 bytes.

Bounded, though: `CRF.LDA`, `C99.LDA` and `372BR.RS4` all verify perfectly in every
tree, so the damage is confined to these four small files, and version-2 supplies
good copies. Recorded as unresolved rather than guessed at.

## 8. Consequences for the plan

- **The Phase 1 gate target changes.** Plan §2 and Phase 1 name the MAME
  `ccastles1` set, following AT6502's precedent. `ccastles1` is **version 1**,
  which cannot be built without reconstructing `CJTB.MAC`. Our gate is
  `ccastles3` / version-3. This does not weaken the gate — the `.LDA` oracle plus
  three documented checksums is a stronger standard than a ROM-set name.
- **Task #12 needs widening.** It says "assemble `CRF.MAC` and `C99.MAC`". The real
  build is four assemblies (`CRF`, `CRP`, `CLS`, `C99`) and two link steps. A
  linker is in scope for Phase 1 and was not previously accounted for.
- Plan §2's "one source file was missing" is resolved: `CJTB.MAC`, absent from
  v1 and v2, present in v3.

## 9. Unverified

- The MAME set names (`ccastles1/2/3`) are asserted from part-number
  correspondence, not checked against a MAME source tree.
- The four PROM files' damage mechanism.
- Whether `CRM.LDA` / `CRN.LDA` / `CLS.SND` / `CWORDS.TXT` / `CEEDUM.MAC` /
  `CRM.MAC` / `CRN.MAC` matter to the build. `372X3.DOC` lists `CJTB.MAC`,
  `CRN.MAC` and `CRM.MAC` among its source files, so they are part of *some*
  procedure, but `CRF.MAC`'s include closure does not reach them.
- `372X3.DOC` lists an include named `CTN.MAC`, which exists in no tree. Almost
  certainly a typo for `CIN.MAC`, which occupies the same position in the v1 and
  v2 lists and does exist. Not consequential, but recorded.
- The `022X*.DAT` verification-control files were read as the ROM manifest (they
  map `.LDA` file → part number → load address, and corroborate §2 exactly). Their
  full syntax was not otherwise analysed.
