# The WebAssembly target

**The Phase 5 deliverable** (plan §6: *"`wasm.md` on constraints hit and how they
were resolved"*).

The game runs in a browser, and produces the same pictures there as it does
natively — frame for frame, on all five recorded traces, with compiled routines
dispatched or not. Every figure below came out of a command run while writing
this, and each is named beside its number.

## The result

```text
$ node web/harness.mjs --wasm target/wasm32-unknown-unknown/release/chill65_wasm.wasm \
      --prog target/boot-artefacts/prog.bin --data target/boot-artefacts/data.bin \
      --trace traces/<t>.trace --expect target/wasm-check/<t>.hashes

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

13,492 frame comparisons, each against the stream `ccnative hashes` printed from
the native interpreter. This is the same discipline as the two phases before it:
Phase 3 compared us against MAME and a verilated MiSTer core, Phase 4 compared
dispatched against interpreted, Phase 5 compares WebAssembly against native. One
seam, a new implementation on one side of it each time.

Dispatch is checked against the *same* expected stream rather than reported
separately, deliberately. A compiled routine that changes the picture is a bug
whichever target it runs on.

## 1. Constraints hit, and how they were resolved

### There is no operating system

`wasm32-unknown-unknown` has no files, no subprocesses and no clock.
`chill65-diff` builds the ROM images by **running the assembler as a
subprocess** (`images.rs`, `Command::new("cargo")`), which cannot happen in a
browser.

So the images arrive as bytes. The module exports `prog_ptr()` and `data_ptr()`,
which return pointers to static 24,576- and 16,384-byte staging buffers inside
its own linear memory; JavaScript writes the images there and calls `boot()`.

That constraint and the distribution posture point the same way, which is worth
noticing. Plan §9 forbids game-derived bytes in this repository, so a module that
*could* embed the ROM still must not. Being unable to load one is the constraint
enforcing itself.

The same reasoning removed host time from the equation. `chill65-runtime` has no
time source and does not get one; wall-clock measurement lives in `ccnative` on
the native side and in `performance.now()` on the browser side. The crate that
has to build for `wasm32` stays free of anything the target lacks.

### Plan §4's structured control flow

Plan §4 exists because WebAssembly permits no arbitrary jumps: control flow must
nest, so irreducible flow forces "a `loop`/`match` dispatch machine that LLVM
cannot optimise across".

Phase 4 built exactly that machine — `emit.rs`'s `Bucket::StateMachine`, a `loop`
around `match block` — and `gate4.md` named it as the largest piece of work
Phase 5 inherits. **Phase 5 measured it rather than replacing it.** Sections 2
and 4 are that measurement. Replacing it is a decision to take with the numbers
in hand, and the numbers now exist.

### No `wasm-bindgen`

`wasm-bindgen` and `wasm-pack` are both installed on the machine this was built
on. Neither is used. `README.md` closes on "No third-party crate dependencies",
and that does not get suspended for the last crate in the workspace.

What is exported instead is a plain C ABI — `#[no_mangle] extern "C"`, integers
and pointers into linear memory. The cost is writing the JavaScript side by
hand, about 60 lines of it. The benefits are concrete: the module needs **no
imports at all**, which `web/smoke.mjs` asserts rather than assumes; there is no
generated glue to keep in step with the Rust; and any embedder — Node, a browser,
anything with a WebAssembly runtime — can call it with no toolchain.

### `u64` arrives as a *signed* BigInt

`frame_hash()` returns `u64`. WebAssembly has only `i64`, and Node hands it to
JavaScript as a **signed** `BigInt`, so any hash with its top bit set arrives
negative: `-55eca3062ff75e25` where the native side prints `aa135cf9d008a1db`.

About half of all hashes have that bit. The first 162 frames of attract mode
happen not to, so this presented as *"diverged at frame 162"* — which is also the
frame `gate3.md` reports the MAME divergence at. Pure coincidence, and a
disconcerting one for a minute.

`BigInt.asUintN(64, …)` reinterprets it. The lesson is that a comparison across
a language boundary can fail on *formatting* while both sides are perfectly
correct, and a divergence report that names a frame number invites you to go
looking at the machine instead.

### Typed-array views die when memory grows

Every `Uint8Array` over `exports.memory.buffer` is invalidated the moment the
module allocates — and the module allocates a `Vec` for the framebuffer on every
render. A cached view silently reads a detached buffer.

Sharper than the usual advice to re-derive views: this line **throws**.

```js
new Uint8Array(wasm.memory.buffer, wasm.prog_ptr(), 24576)   // TypeError
```

JavaScript evaluates arguments left to right. `wasm.memory.buffer` is captured
first; then `prog_ptr()` runs, and the lazy initialisation behind it grows memory
to make room for the machine, detaching the buffer already captured. Take the
pointer first, read `.buffer` second — always in that order.

### Two smaller ones

An instantiated module's `exports` object is **frozen**, so a helper cannot be
attached to it.

And a display frame is not a game frame. The board runs at 61.035 Hz
(`frame.rs`: 1,250,000 CPU cycles/second ÷ 20,480 per frame) and a monitor runs
at whatever it runs at, so `web/app.js` drives game frames from accumulated real
time, capped at four per animation frame — otherwise a backgrounded tab returns
and tries to catch up on ten minutes at once.

## 2. Reducibility — `codegen-readiness.md`'s closing question, answered

`codegen-readiness.md` measured 266 hand-written branches, 30.4% of the game's
control flow, and closed by saying whether they are mostly *reducible* was "a
separate and more important question for the WASM target… It should be
[answered], before Phase 5 is planned." It was not answered then. It is now.

`chill65-asm` builds each routine's control-flow graph with **the same leader
rule the state machine uses** — measuring a different graph would answer a
different question — computes dominators, and calls a routine reducible when
every retreating edge's target dominates its source.

```text
$ CHILL65_ANALYSE=1 cargo run -p chill65-asm -- CRF.MAC CRP.MAC CLS.MAC \
      -I .../version-3 -I ...

O3: branches by origin, of 876:
      hand-written      :   266  (30.4%)

Reducibility of the hand-written control flow (101 routines with a bare branch)
    aggregate: 266 hand-written branches inside routines
      reducible   :  99 routines,  226 branches (85.0%)
      IRREDUCIBLE :   2 routines,   40 branches (15.0%)
```

The 266 arrived at by walking graphs agrees exactly with the 266 counted from
macro-expansion chains at emission — two independent routes to the same number.

**Five sixths of the hand-written control flow is reducible.** It could, in
principle, be rebuilt as nested `loop`/`if` with no dispatch at all, which is
what plan §4 ranks above the fallback.

The 15% that resists is **two routines**:

```text
    $DETCT   F697   55 blocks   5 loops   38 bare branches   IRREDUCIBLE
                    loops entered at more than one block: F6DD -> F6E0
    EN.COL   B10D   40 blocks   1 loop     2 bare branches   IRREDUCIBLE
                    loops entered at more than one block: B1F2 -> B1FB
```

`$DETCT` carries 38 bare branches by itself — 14% of the game's total in one
routine. Everything else, including `MN.ST` at 37 blocks and 10 loops, is
reducible.

**This is a finding, not work done.** Structure recovery was not attempted in
Phase 5 and `emit.rs` was not touched. What the measurement establishes is that
such a pass would have something to work on, and that a dispatch would still be
needed for two routines — so it is a *reduction* of the state machine's reach,
never an elimination of it.

## 3. Size

```text
$ ls -l target/wasm32-unknown-unknown/release/chill65_wasm.wasm target/release/ccnative

499561  chill65_wasm.wasm     (built with CHILL65_CORPUS)
850192  ccnative
```

The two are not comparable and are shown together only because both get called
"the binary". They contain different things.

**`chill65_wasm.wasm`, 499,561 bytes** — the 6502 interpreter, the Crystal
Castles board model, the compiled routine registry, and the C ABI. Built without
a corpus the same module is **347,503 bytes**, so **152,058 bytes of it is
compiled game code** — 22 routines from `routines.txt`, expanded into 175
dispatch entries because a state-machine routine registers every block so the
dispatch can resume it.

**`ccnative`, 850,192 bytes** — a native host tool that carries the entire
differential harness with it: the trace parser, the MAME and MiSTer oracle
drivers, ZIP and CRC-32 writers, the `.LDA` decoder, routine attribution. Almost
none of that is the machine.

**`wasm-opt` is not installed and was not used.** No post-processing of any kind
was applied — no size pass, no stripping beyond what `--release` does. Recorded
as an open item: the figure above is what `rustc` emits, and it is the number a
later size pass would be measured against.

## 4. Performance

`gameplay.trace`, 2,640 frames — the longest trace, and at 49.8% the *lowest*
compiled share of the five, so it is the least favourable case for dispatch.

```text
$ cargo run --release -p chill65-native --bin ccnative -- bench --trace traces/gameplay.trace
  interpreted   2640 frames   448.5 ms   5886.0 frames/sec
  dispatched    2640 frames   865.3 ms   3050.8 frames/sec

$ node web/harness.mjs --bench --wasm ... --trace traces/gameplay.trace
  interpreted   2640 frames   526.8 ms   5011.5 frames/sec
  dispatched    2640 frames  1052.8 ms   2507.6 frames/sec
```

Against the board's real-time bar of **61.035 Hz**:

| | frames/sec | × real time |
|---|---:|---:|
| native, interpreted | 5886.0 | 96.4× |
| native, dispatched | 3050.8 | 50.0× |
| wasm, interpreted | 5011.5 | 82.1× |
| wasm, dispatched | 2507.6 | 41.1× |

**All four clear real time by a wide margin.** The slowest cell runs the game
forty times faster than the hardware did. Nothing here is a performance problem
in the ordinary sense.

Two findings, and the second is the interesting one.

**WebAssembly costs about 15–18%.** 5011.5 against 5886.0 interpreting (85.1% of
native), 2507.6 against 3050.8 dispatching (82.2%). For an interpreter loop —
branchy, memory-heavy, the worst shape for a sandboxed target — that is a smaller
penalty than plan §4's framing led me to expect.

**Compiled dispatch is roughly twice as slow as interpreting, on both targets.**
Native 1.93×, wasm 2.00×. Compiling 49.8% of executed instructions into Rust made
the program half as fast.

That last figure needs stating carefully, because it is easy to misread.

`gate4.md` claimed only that the translation was *faithful*, said explicitly that
"nothing here measures wall clock", and predicted the state-machine lowering was
"the wrong shape for it". This is that prediction confirmed with a number on it.

But the number does **not** say WebAssembly is the problem. The penalty is the
same on both targets — 1.93× native against 2.00× wasm — so it is not something
LLVM does well natively and badly through a sandbox. It is the shape of the
lowering itself. Every block boundary in a state-machine routine is an assignment
to `block` and a trip round `match`, where the interpreter would simply have
advanced a program counter; and the `Compiled::enter` dispatch does a linear scan
of 175 entries at every instruction boundary the interpreter offers it.

So plan §4's warning is right about the mechanism and, on this evidence, wrong
about the venue: `loop`/`match` is a poor shape *everywhere*, and WebAssembly
merely inherits it. Section 2 says 85% of that machine could become real
`loop`/`if`. Whether doing so recovers the factor of two is unmeasured and
untried, and belongs to whoever picks up Phase 6.

## 5. Scope, and distribution

**Motion objects are not modelled** — and on a page with a picture on it, this
is the thing to say first. `gate2.md` §12.1 recorded it, `gate3.md` and
`harness.md` repeat it, and `video.rs` names the arbitration it does not
implement.

The split is visible on screen. The **bitmap** carries the castle and the
crystals, and those render: the game draws them into the framebuffer, and
erases a crystal as it is collected, so they are part of the playfield rather
than sprites over it. The **motion objects** carry the characters — Bentley
Bear and everything chasing him — and none of them appear.

So the page renders the environment, correctly, with nobody in it. That is the
model behaving correctly at its current extent, not a fault in the port and not
anything Phase 5 introduced — the native build draws the same picture, which is
why the two agree frame for frame. But "runs in a browser" must not be allowed
to imply "plays in a browser", and this is the gap between them.

**Audio remains out of scope.** The two POKEYs are modelled as far as the
registers and a real poly17/poly9 LFSR (`hardware.md`), because the game reads
them back and its timing depends on them. No samples are generated and nothing is
played. A subsystem, not a detail.

Windowing is out of scope on the native side, unchanged from Phase 4. The browser
page is the only front end with a picture on it.

**What is committed**, all of it our own work:

```text
crates/chill65-wasm/    the module: Cargo.toml, src/lib.rs
web/smoke.mjs           does it load and run under Node
web/harness.mjs         does it match native, on all five traces
web/index.html          the page
web/app.js              canvas, pointer-lock trackball, keyboard switches
tools/stage-web.sh      assembles target/web from the built parts
```

**What is not, and never will be:** `chill65.wasm` built with a corpus contains
the game's compiled routines and is exactly as game-derived as the ROM; `prog.bin`
and `data.bin` are the ROM images. All three live under gitignored `target/`,
`tools/stage-web.sh` assembles them into `target/web`, and none of it may be
copied out or committed (plan §9). The same posture as every phase before:
**ship the implementation, require the user to provide the game.**

## What this document does not claim

**That it is playable.** Every check above is machine-made: the module builds,
stages, loads under Node, and reproduces native's pictures exactly. Whether the
trackball feels right, and whether a person can play the game, is human-judged —
and with motion objects unmodelled there is nothing yet to play. "Playable" has
been left standing since Phase 2 and Phase 5 does not close it. `gate5.md` states
where it stands rather than assuming it.

**That the wasm build agrees with MAME or the MiSTer core.** It agrees with our
native build, which is a different and smaller claim. The two-pixel `MN.ST`
divergence at frame 162 that both external oracles report is in our model and is
still unexplained; the WebAssembly build reproduces it faithfully, which is
correct behaviour for a port and no comfort at all about the underlying question.
