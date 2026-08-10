# Phase 5 gate — PASSED

The game runs in a browser, and the WebAssembly build produces the native
build's pictures exactly — every frame, every trace, dispatch off and on.

## The ten commands, one run, all exit 0

| # | Command | Result |
|---|---|---|
| 1 | `cargo build` | clean |
| 2 | `cargo test` | **292 passed**, 0 failed, 29 ignored |
| 3 | `cargo test -p chill65-asm --test gate -- --ignored` | **byte-identical, both images** |
| 4 | `cargo test -p chill65-runtime -- --ignored` | 6 passed |
| 5 | `cargo test -p chill65-runtime --test klaus -- --ignored` | 2 passed |
| 6 | `cargo test -p chill65-diff -- --ignored` | 8 passed, both oracles live |
| 7 | `cargo test -p chill65-native -- --ignored` | **66.0% compiled**, majority held |
| 8 | `cargo build -p chill65-wasm --target wasm32-unknown-unknown --release` | 499,561 bytes |
| 9 | `ccnative hashes` + `node web/harness.mjs`, five traces | **13,492 frames identical to native** |
| 10 | `node web/smoke.mjs`, `bash tools/stage-web.sh` | both exit 0 |

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

Every figure identical to `gate2.md`, `gate3.md` and `gate4.md`. Phase 5 added a
crate, two Node scripts, a web page and a CFG analysis, and moved none of them.

The test count rose from 287 to 292: four reducibility tests in `chill65-asm`
and one in `chill65-wasm`, all corpus-free.

### 6 — Phase 3 unregressed, both oracles exercised

```text
clean vs clean:  identical over 600 frames
clean vs faulty: diverged at frame 419: LN.F1 [CRF.MAC] (85 pixels), …
mame: 0.276 (unknown)
mame determinism:   500 frames identical across two runs
mister determinism:  60 frames identical across two runs
ours vs mame:    diverged at frame 162: MN.ST [CRF.MAC] (2 pixels)
ours vs mister:  diverged at frame 162: MN.ST [CRF.MAC] (2 pixels)
ROM set:         11 of 11 devices match MAME's CRC-32
```

**One honest detail.** The command as written passes with the MiSTer core live
and the MAME test *self-skipping*, because MAME is gated on `CHILL65_MAME` and
that command sets only `CHILL65_CORPUS`. Reported here as skipped would have been
a gap against `gate4.md`'s "both oracles live", so it was re-run as
`CHILL65_MAME=$(which mame) cargo test -p chill65-diff -- --ignored`; the lines
above are from that run. Both oracles still independently report the same
two-pixel difference at frame 162 — **since explained**, and not a fault in the
machine model. See `harness.md` §12 and the note below.

### 7 — the majority still holds

```text
coin-start.trace     900 frames    5,559,710 instructions   76.4% compiled
gameplay.trace      2640 frames   15,844,630 instructions   49.8% compiled
idle-attract.trace   600 frames    3,950,048 instructions   69.9% compiled
selftest.trace       600 frames    3,677,393 instructions   57.8% compiled
wrap-stress.trace   2006 frames   11,491,747 instructions   84.4% compiled

aggregate: 26,734,538 of 40,523,528 executed instructions compiled (66.0%)
```

### 9 — the Phase 5 metric

```text
coin-start     interpreted   900 frames identical to native
               dispatched    900 frames identical to native
gameplay       interpreted  2640 frames identical to native
               dispatched   2640 frames identical to native
idle-attract   interpreted   600 frames identical to native
               dispatched    600 frames identical to native
selftest       interpreted   600 frames identical to native
               dispatched    600 frames identical to native
wrap-stress    interpreted  2006 frames identical to native
               dispatched   2006 frames identical to native
```

## What this gate asserts

**The WebAssembly module's per-frame FNV-1a frame-hash stream equals the native
interpreter's, over all five committed traces, with compiled dispatch both off
and on.**

Stated precisely, because each clause is load-bearing:

- **Per-frame, not final.** A hash after every frame, compared position by
  position, so a divergence is dated rather than merely detected.
- **`Machine::frame_hash`** — FNV-1a over the framebuffer's palette indices,
  which is the one hash a `wasm32` target can produce without `chill65-diff`
  (which shells out to `cargo` and cannot run in a browser).
- **All five committed traces**, 6,746 frames each way, 13,492 comparisons.
  Not a sample.
- **Both dispatch modes**, checked against the *same* expected stream rather
  than against each other. A compiled routine that changes the picture is a bug
  whichever target it runs on.
- **Reproducible** by the commands in row 9, from a clean tree.

This is the same discipline as the two phases before it, with the seam moved.
Phase 3 put MAME and a verilated MiSTer core on the far side; Phase 4 put the
compiled dispatch there; Phase 5 puts WebAssembly there.

## What it deliberately does not assert

**That the wasm build is fast enough.** Performance is *reported*, not gated.
All four measured cells clear real time by 41× or better, but no assertion
depends on it — a gate that fails on a slow machine tests the machine.

**That it is playable.** See below; that is human-judged and it is not claimed.

**That the wasm build agrees with MAME or the MiSTer core.** It agrees with our
native build, which is a smaller and different claim. Both external oracles
still report the two-pixel `MN.ST` difference at frame 162, and the WebAssembly
build reproduces it faithfully — correct behaviour for a port.

**Note added after this gate ran.** That divergence is no longer unexplained.
It is the power-on RAM test at `EBDE`–`EBFA` writing `FF` to every byte of RAM
— the bitmap included — and clearing it again; our framebuffer is a snapshot of
RAM at the frame boundary, so it catches whichever byte is mid-test, while MAME
and the core scan out and essentially never coincide with that byte's 13-cycle
window. The lit byte walks through memory exactly as the test does. Not a fault
in the machine: an artefact of snapshot-versus-scanout extraction, which will
recur wherever the game writes a transient into bitmap RAM. `harness.md` §12
sets it out in full, with the evidence and the wrong fix it rules out.

**That structure recovery was done.** It was measured, not written. See below.

## Reported, not gated

### Reducibility — `codegen-readiness.md`'s closing question, answered

`CHILL65_ANALYSE=1 cargo run -p chill65-asm -- CRF.MAC CRP.MAC CLS.MAC -I …`

```text
aggregate: 266 hand-written branches inside routines
  reducible   :  99 routines,  226 branches (85.0%)
  IRREDUCIBLE :   2 routines,   40 branches (15.0%)
```

The 266 reached by walking control-flow graphs agrees exactly with the 266
counted from macro-expansion chains — two independent routes to one number. The
15% that resists is two routines, `$DETCT` (38 bare branches on its own) and
`EN.COL`. `emit.rs` was not touched; this is a finding for whoever picks up
Phase 6.

### Size

`ls -l target/wasm32-unknown-unknown/release/chill65_wasm.wasm target/release/ccnative`

```text
499561  chill65_wasm.wasm   with a corpus
347503  chill65_wasm.wasm   without one — so 152,058 bytes is compiled game code
850192  ccnative            a host tool carrying the whole diff harness
```

**`wasm-opt` is not installed and was not used.** No post-processing of any kind.
An open item, and the baseline a later size pass would be measured against.

### Performance

`ccnative bench` and `node web/harness.mjs --bench`, both on `gameplay.trace` —
2,640 frames, and at 49.8% the *lowest* compiled share of the five, so the least
favourable case for dispatch.

| | frames/sec | × real time (61.035 Hz) |
|---|---:|---:|
| native, interpreted | 5886.0 | 96.4× |
| native, dispatched | 3050.8 | 50.0× |
| wasm, interpreted | 5011.5 | 82.1× |
| wasm, dispatched | 2507.6 | 41.1× |

**WebAssembly costs 15–18%.** Less than plan §4's framing suggested for an
interpreter loop.

**Compiled dispatch is about twice as slow as interpreting — on both targets**,
1.93× native against 2.00× wasm. `gate4.md` claimed only faithfulness, said
outright that nothing measured wall clock, and predicted the state-machine
lowering was "the wrong shape for it". That is now confirmed with a number. The
penalty being the *same on both targets* is the informative part: plan §4's
`loop`/`match` warning is right about the mechanism and, on this evidence, wrong
about the venue. It is a poor shape everywhere, and WebAssembly inherits it.

`wasm.md` argues both findings in full.

## The human-judged artefact

**A human has looked.** Matt served `target/web`, opened the page, inserted a
coin and started a game.

What he reports seeing: **the playing environment and the crystals render; there
are no characters.** That is the model behaving correctly at its current extent.
The bitmap carries the castle and the crystals, and the game draws them —
erasing each crystal as it is collected — so they appear. Bentley Bear and
everything chasing him are **motion objects, which are not modelled at all**
(`gate2.md` §12.1, `hardware.md`, `video.rs`), so they do not.

That the coin was accepted and the game started is a real result and worth
naming: it exercises the whole path — page, module, staged images, keyboard
switch *levels* reaching the game's own coin routine, and the game's state
machine advancing out of attract mode. None of that was checked by any automated
test here.

**"Playable" is still not claimed, and is not closed by this.** With no
characters on screen there is nothing yet to play. The milestone has been open
since Phase 2 and Phase 5 does not close it; what Phase 5 establishes is that
when motion objects are modelled, they will appear in a browser as faithfully as
everything else does.

## Distribution

`git status --short` is **empty** and nothing game-derived is tracked: no
`.wasm`, no `.bin`, no `.hashes`, no framebuffers.

Committed for Phase 5, all our own work:

```text
crates/chill65-wasm/Cargo.toml, src/lib.rs   the module and its C ABI
crates/chill65-asm/src/reduce.rs             the reducibility measurement
crates/chill65-asm/tests/reduce.rs           its two hand-authored fixtures
crates/chill65-native/src/bin/ccnative.rs    `hashes` and `bench` added
web/smoke.mjs                                does it load and run under Node
web/harness.mjs                              does it match native, five traces
web/index.html, web/app.js                   the page
tools/stage-web.sh                           assembles target/web
wasm.md, gate5.md                            the write-up and this gate
```

Plus one line each in the workspace `Cargo.toml` and `chill65-asm`'s `lib.rs`,
and a hook into `main.rs`'s existing `analyse`.

Not committed, and never: `chill65.wasm` built with a corpus holds the game's
compiled routines and is as game-derived as the ROM; `prog.bin` and `data.bin`
are the ROM images; the `.hashes` streams describe the game's output. All live
under gitignored `target/`, and `tools/stage-web.sh` assembles them into
`target/web` without copying anything out of it.

The three `.MAC` files that *are* tracked — `chill65-asm/tests/fixtures/hll.MAC`
and `chill65-native/fixture/{fixture,hll}.MAC` — are original programs written
for this repository, defining constructs of the same names HLL65F uses and
nothing more. They contain no game code, which is what lets `cargo test`
exercise the whole pipeline, including the WebAssembly module, on a machine with
no corpus at all.
