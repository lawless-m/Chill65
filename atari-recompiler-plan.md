# Atari 6502 Arcade Static Recompiler

**Working title:** unnamed. **First target:** Crystal Castles (Atari, 1983).
**Deliverable:** a toolchain that translates original Atari arcade assembly source into Rust, plus a runtime and per-game hardware models, targeting native and WebAssembly.

---

## 1. What this is

A source-level static recompiler. The input is the original Atari assembly source (not ROM bytes, not a disassembly), the output is Rust that can be compiled natively or to WASM and run in a browser without emulating a 6502 at the instruction level.

The reusable artefact is the toolchain, not the port. Crystal Castles is the first target because it is the one game in the archive already proven to reassemble byte-identically, which gives a hard correctness gate from day one.

### Why source rather than ROM

Every static recompiler that starts from a binary emits a flat sea of gotos, because control flow is the only thing recoverable from a branch graph. The Atari source uses a macro package (HLL65F) providing run-time nested if/then structures — the original programmers wrote structured control flow, and it survives in the source. Lowering *that* gives real `if`/`else` and loops in the output.

This is not a nicety. It is the difference between a WASM build that compiles well and one that degenerates into a dispatch state machine (see §4).

### Non-goals

- Cycle-exact fidelity as an acceptance standard. Fidelity tooling is built as a *debugging* instrument, not a bar to clear.
- Preservation-grade emulation. MAME and the MiSTer core already exist and do this properly.
- Distributing game code. See §9.

---

## 2. Established facts

Verified before planning; each has a bearing on the design.

| Fact | Source | Consequence |
|---|---|---|
| Original Atari source for Crystal Castles is public | `github.com/historicalsource/crystal-castles` | No reverse engineering required |
| Atari used an in-house VAX assembler with non-standard syntax (e.g. addressing mode written as `LDA ZX,$PSTSL`) | DanBoris, AtariAge | No off-the-shelf assembler works; front end must be bespoke |
| AT6502 (C#) reassembles the source to the MAME `ccastles1` ROM set byte-for-byte | `github.com/danlb2000/AT6502` | A correctness oracle exists for the front end |
| One source file was missing and reconstructed from ROMs | DanBoris | Source integrity is not assumed |
| Source uses HLL65F macro package with nested if/then | DanBoris blog | Structured control flow is recoverable |
| Castle/wave data is binary in `.DAT` files, parsed at run time; a castle is a "city", drawn by `CT.DRW` in `CCT.MAC` | Dan's Devlog | Data path is separate work from the code path |
| Framebuffer is 256×232 and is *not* cleared per frame — only between levels | Dan's Devlog | Affects video model design |
| MiSTer FPGA core exists (Enceladus, 2022): 10 MHz game clock, 5 MHz pixel, 320 horizontal / 252 visible, 256 vertical / 232 visible, 61.03 Hz | `MiSTer-devel/Arcade-CrystalCastles_MiSTer` | Open Verilog reference for the custom video hardware |
| The core's author flags the "Potato" custom chip as not fully characterised — scrolling and memory addressing uncertain, a 0–23 clamp queried, cocktail sprite positioning wrong | ibid., outstanding-work notes | A genuinely open question sits in the middle of this |
| Archive is uneven: some titles corrupt or incomplete (Space Duel), some are ROM dumps only (Dig Dug); dialects differ enough that a Joust-targeted tool could not be retargeted to Centipede or Crystal Castles | arcade-museum.com forums | Multi-game scope is unknown until surveyed |
| AT6502 hit unimplemented pseudo-ops immediately on Asteroids Deluxe | DanBoris | Dialect variation is real, not theoretical |

---

## 3. Architecture

Five components. The split between shared and per-game is the whole point of the project.

**Front end (shared).** Lexer, parser and macro expander for the Atari assembler dialect(s). Either reuse AT6502's front end or reimplement in Rust — see open question O1. Must preserve comments and symbol names; they are the raw material for readable output.

**IR (shared).** A language-neutral intermediate representation sitting between front end and emitter. This is the load-bearing design decision. It must carry:
- Structured control flow where HLL65F provides it, falling back to a raw CFG where the source uses bare branches
- Symbol names and source comments, for provenance in the output
- An explicit code/data distinction (the source declares data; do not re-infer it)
- Jump tables as first-class labelled data rather than computed targets

The IR keeps the backend choice reversible. Emitting Rust first does not preclude a C or C# backend later.

**Emitter (shared).** IR to Rust. Function per routine — never one large function; LLVM handles enormous basic-block soup badly and compile times become unmanageable.

**Runtime crate (shared).** 6502 semantics (registers, flags, addressing helpers), cycle accounting, interrupt scheduler, POKEY. Cycle accounting is not optional even given the relaxed fidelity goal: a 1983 coin-op is written against vertical blank and hardware timers, and something must fire those interrupts at the right points.

**Hardware model (per game).** Video, I/O, game-specific chips. For Crystal Castles this is the custom bitmap logic and the Potato chip, with the MiSTer Verilog as the behavioural reference.

---

## 4. The WASM constraint

WebAssembly permits only structured control flow — blocks, loops, and branches to enclosing labels. There is no arbitrary jump, and Rust has no `goto`. Transpiled assembly is a raw CFG which frequently has multiple entry points into a single loop; irreducible control flow is the pathological case, forcing a `loop`/`match` dispatch machine that LLVM cannot optimise across.

Mitigations, in order of preference:

1. **Carry HLL65F structure through.** Where the source has structured control flow, emit structured Rust directly. Expected to cover the bulk of the code.
2. **Relooper fallback.** For routines written in bare branches, apply a relooper-style algorithm to recover structure.
3. **Dispatch table.** For genuinely computed jumps, a table lowering to `call_indirect`. Contained, and the residue should be small.
4. **Interpreter fallback.** Anything that resists all of the above stays interpreted. This is a feature, not a defeat — see §5.

Measure the ratio of routines landing in each bucket during Phase 2; it is the single best predictor of how the WASM build will behave.

---

## 5. The creep line

The central working method. An interpreter and compiled routines share one machine state, and routines migrate from the former to the latter one at a time.

Consequences:
- The game runs from the end of Phase 2 onwards. There is never a non-working build.
- The blast radius of any bug is one routine.
- No big-bang integration.
- Unresolvable indirect jumps degrade gracefully instead of blocking the build.

This also means the interpreter is permanent infrastructure, not scaffolding. Build it accordingly.

---

## 6. Phases

### Phase 0 — Survey and gates
Cheap, and may reshape everything downstream.

- **0.1 Self-modifying code gate.** Read the Crystal Castles source for self-modifying code. This is the one pattern that genuinely breaks static recompilation. Go/no-go before any tooling is written.
- **0.2 Source integrity.** Establish which of `version-2` / `version-3` corresponds to the `ccastles1` ROM set. Verify completeness against DanBoris's documented modifications and reconstructed file. Pick one version and stay on it.
- **0.3 Archive inventory.** Across `historicalsource`, catalogue: which repos contain complete buildable assembly (versus ROM dumps or corrupt material), which assembler dialect each uses, what per-game macro libraries exist, which are raster versus vector hardware. Output: a table. This decides whether "multi-game" means five titles or twenty-five.
- **0.4 Data format reconnaissance.** Read `CT.DRW` in `CCT.MAC` and establish how the `.DAT` wave data is consumed.

**Deliverable:** `inventory.md`, `integrity.md`, and a go/no-go on 0.1.

### Phase 1 — Front end and reassembly
- Parser, macro expander, dialect handling for Crystal Castles.
- **Gate:** reproduce the `ccastles1` ROM set byte-for-byte. This proves the front end understands the source exactly, before any semantic work begins.
- Emit an IR dump alongside the ROM output for inspection.

**Deliverable:** front end, byte-identical ROM reproduction, `dialect.md` documenting the assembler's syntax and pseudo-ops.

### Phase 2 — Runtime and interpreter
- 6502 core, cycle accounting, interrupt scheduler.
- Video model for Crystal Castles, using the MiSTer Verilog as behavioural reference. Note the framebuffer is not cleared per frame.
- POKEY (×2). Borrow or port an existing model; this is not the interesting part.
- Trackball input: raw deltas (evdev / Raw Input / SDL relative mode), accumulate between frames, present once per frame, replicate the original counter's saturation behaviour. No OS pointer acceleration.
- **Gate:** game runs, fully interpreted, playable.

**Deliverable:** playable interpreted build; `hardware.md` documenting the video and Potato behaviour as understood.

### Phase 3 — MAME differential harness
- Drive MAME and the interpreted build with an identical recorded trackball trace; diff framebuffers per frame.
- Purpose is debugging, not certification: the useful output is "diverged at frame 4,812, routine X".
- Build this *before* the emitter. It makes every subsequent decision cheap to check.

**Deliverable:** harness, plus a corpus of recorded input traces.

### Phase 4 — Rust emitter and migration
- IR to Rust, function per routine.
- Migrate routines from interpreter to compiled, one at a time, re-diffing after each.
- Track the bucket ratio from §4 as migration proceeds.

**Deliverable:** majority-compiled native build; `codegen.md` covering the lowering strategy and what resisted it.

### Phase 5 — WASM target
- Browser build. Canvas or WebGL for the framebuffer, Web Audio for POKEY, Pointer Lock for raw mouse deltas.
- Compare compiled output size and performance against the native build.

**Deliverable:** browser build; `wasm.md` on constraints hit and how they were resolved.

### Phase 6 — Second game
**Decided: Space Duel.** This section originally recommended "a vector title
(Asteroids Deluxe, Black Widow, Space Duel — subject to Phase 0.3 integrity
findings)" and deferred the choice until after Phase 4, when the cost of a
per-game hardware model would be known from experience rather than estimated.
Phases 4 and 5 are done, along with motion objects and POKEY audio, so that
experience exists and the choice is made.

Space Duel on the evidence in `inventory.md` §7, which reached the same
conclusion independently:

- It has **HLL65F**, so the structured-control-flow path applies. Asteroids
  Deluxe does not, which makes it the weakest of the three despite being listed
  first here.
- It has the **shared vector macros** on the `VGMC`/`VGUT` branch used by four
  other titles. Black Widow's `VGMC16`/`VGUT16` differ, so Space Duel sits on
  the more reusable side.
- It ships **`.LDA` files**, which are oracles — the Phase 1 gate for Crystal
  Castles was byte-identical output against exactly this kind of artefact.
- 184 KB, 26 `.MAC`, 24 `.DAT`, plus a `revision/` subtree.

Two things to settle before relying on it, neither blocking the decision:

1. **The corruption caution is unresolved.** This plan warned the tree might be
   corrupt; `inventory.md` found that "not corroborated by the file listing" but
   was explicit that file listings cannot detect corruption. It needs a content
   check, which is the natural first task.
2. **It was built on Asteroids Deluxe** (`AST2RD.MAC`, `ASTRD2.MAC`, `A2*.MAC`),
   so expect inherited 1981-era dialect alongside the 1983 HLL65F — the archive
   is two dialects and this title may straddle them.

Rationale: same 6502 and POKEY, but rendering goes through a vector generator — a small, well-documented display-list processor with no custom pixel hardware to model. In a browser that is line segments on a canvas. This proves engine generality for a fraction of the hardware work, and vector games look genuinely good with modern line rendering. Centipede would exercise the raster path harder, and is the natural third.

**Deliverable:** second game running; a written account of what had to move from per-game into shared.

---

### Parked: Battlezone as an alternative second target

Not decided. Noted here so the option isn't lost.

Battlezone (1980) is a three-processor machine: 6502 for gameplay, the AVG for display, and a "math box" built from four cascaded AMD 2901 bit-slices (16-bit ALU at 3 MHz) for the 3D transforms. The transpiler addresses only the 6502; the math box is an unrelated instruction set and would be modelled as a per-game coprocessor. MAME's approach — implement the ~32 operations functionally rather than executing the microcode — is almost certainly right here, and trivial in Rust.

Attractions: the math box is shared with Red Baron and Tempest, and the AVG with Black Widow and Space Duel, so this unlocks the largest cluster in the archive. It also proves the architecture generalises past "6502 plus a framebuffer" in a way Asteroids Deluxe would not.

Costs and cautions:
- Source was released October 2021 (with math box internal documentation) — location to be confirmed in Phase 0.3. It is a 1980 title against Crystal Castles' 1983, so expect assembler dialect drift.
- The 6502 does not always wait on the math box busy flag — it sometimes delays a fixed number of cycles and reads, and starting a new operation halts the previous one mid-calculation. Timing-coupled cross-processor behaviour, and exactly what breaks when the CPU is compiled away and runs unthrottled. Good stress test for the scheduler; also a real risk.
- Different vector hardware from Asteroids Deluxe (analogue vs digital generator).

Reference material is unusually good: Andy McFadden's annotated SourceGen disassembly (6502disassembly.com/va-battlezone/), updated with source references in 2022, covering the 6502 code, vector commands and math box interface; and brouhaha's `mathdis` for the microcode ROMs if faithful execution is ever wanted.

Decide after Phase 4, when the cost of a per-game hardware model is known from real experience rather than estimated.

---

## 7. Open questions

- **O1.** Reuse AT6502's C# front end, or reimplement in Rust? Reuse saves weeks of the least rewarding work (VAX-dialect parsing, macro expansion) at the cost of a two-language project. Reimplementation gives a single-language toolchain. Decide after Phase 0.3, when the dialect variation across the archive is known — if every game needs front-end work, owning it in Rust is worth more.
- **O2.** Does the source contain self-modifying code? Phase 0.1.
- **O3.** How much of the code is HLL65F-structured versus bare branches? Determines how much relooper work is needed. Measurable at the end of Phase 1.
- **O4.** Is the Potato chip's behaviour fully derivable from the game source plus the MiSTer implementation, or does it need hardware observation? The FPGA author left it open. Having the source on one side and a Verilog attempt on the other is a better position than either party had alone — but it may not be sufficient.
- **O5.** Does the `.DAT` wave data warrant a standalone level viewer? It was asked for on AtariAge in 2022 and never built. Small, independently useful, and good validation of the data parser.

---

## 8. Risks

**Archive thinner than hoped.** If Phase 0.3 finds only a handful of complete, dialect-compatible sources, the multi-game framing weakens. Mitigation: Phase 0.3 is deliberately first and cheap. If it comes back thin, reconsider scope before building anything.

**Irreducible control flow more common than expected.** Would degrade the WASM build badly. Mitigation: measured in Phase 1, not discovered in Phase 5; interpreter fallback bounds the damage.

**Timing-dependent code.** Raster-position-dependent effects and POKEY timer interactions are the classic source of subtle breakage. Mitigation: cycle accounting from the start, plus the frame diff to localise it.

**Scope creep into emulator territory.** The honest description of the output is "an emulator with the CPU compiled away". That is legitimate and is what static recompilation means everywhere it has been done — but the project should not drift into general-purpose emulation, which MAME already does better.

---

## 9. Distribution posture

The `historicalsource` material is archived donated material, not a licensed release, and Atari is an active rights holder. The toolchain, runtime and hardware models are entirely original work and publishable. Emitted game code is not.

Therefore: ship the toolchain, and have the user supply their own source. This is the same posture the MiSTer core takes in shipping without ROMs and requiring the user to provide them — a pattern that community understands and respects. It also means the interesting artefact is the public one.

---

## 10. Suggested order of work

1. Phase 0.1 self-modifying code gate. Stop here if it fails.
2. Phase 0.2 version selection and integrity.
3. Phase 0.3 archive survey — informs O1 before any front-end commitment.
4. Phase 1 front end to byte-identical ROMs.
5. Phase 2 runtime and interpreter to playable.
6. Phase 3 harness before writing the emitter.
7. Phase 4 migration.
8. Phase 5 WASM.
9. Phase 6 second game.

Phases 0 and 3 are the two most likely to be skipped and the two that most repay being done first.

---

## 11. References

- Source archive: `github.com/historicalsource/crystal-castles`
- AT6502 assembler (C#): `github.com/danlb2000/AT6502`
- DanBoris write-ups: `dansdigitalarchaeology.blogspot.com` — source overview, HLL65F macro package
- Playdate port devlog (source-level reimplementation, useful on data formats): `dansdevlog.wordpress.com`
- MiSTer core: `github.com/MiSTer-devel/Arcade-CrystalCastles_MiSTer`
- Hardware schematics and pinouts: arcade-museum.com Crystal Castles entry
- MAME `ccastles` driver — behavioural reference; check licence terms before reading closely with intent to reimplement
