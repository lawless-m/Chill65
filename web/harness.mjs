// Does the WebAssembly build produce the same pictures as the native one?
//
// This is Phase 5's turn at the same discipline every phase has used. Phase 3
// drove our runtime and MAME from one recorded trace and compared; Phase 4 drove
// the interpreter and the compiled dispatch and compared; this drives the wasm
// module and compares it against the native interpreter's hash stream. Same
// seam, same traces, a new implementation on one side of it.
//
//   node web/harness.mjs --wasm W --prog P --data D --trace T --expect H [--frames N]
//   node web/harness.mjs --wasm W --prog P --data D --trace T --bench   [--frames N]
//
// `--expect` is the output of `ccnative hashes`, one 16-hex-digit frame hash per
// line. The whole trace is played twice — once interpreting, once dispatching
// compiled routines — and **both** streams must equal it. Dispatch is not a
// separate result to be reported; a compiled routine that changes the picture is
// a bug whichever target it runs on.
//
// Exit 0 identical, 1 on divergence, 2 on a usage or IO error.
//
// No npm packages. Same posture as everything else here.
//
// # Why the ROM images are arguments
//
// Because they cannot be anything else. `wasm32-unknown-unknown` has no files
// and no subprocesses, so the module cannot assemble or load them; and this
// repository contains no game bytes and never will (plan §9). Both images are
// produced under `target/` by `ccnative hashes`, which is the command that
// writes the expected stream anyway.

import { readFileSync } from 'node:fs';

/// The switch names a trace may use, in the order `chill65-wasm`'s
/// `set_switch` numbers them — which is the order `ALL_SWITCHES` serialises
/// them in (crates/chill65-diff/src/trace.rs). The two tables have to agree and
/// there is no way to check that from here, so it is written out in full rather
/// than abbreviated.
const SWITCHES = [
  'Start1',
  'Start2',
  'SelfTest',
  'Slam',
  'CoinAux',
  'CoinLeft',
  'CoinRight',
];

class Usage extends Error {}

function parseArgs(argv) {
  const opts = { frames: null, bench: false };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    const value = () => {
      const v = argv[++i];
      if (v === undefined) throw new Usage(`${arg} needs a value`);
      return v;
    };
    switch (arg) {
      case '--wasm': opts.wasm = value(); break;
      case '--prog': opts.prog = value(); break;
      case '--data': opts.data = value(); break;
      case '--trace': opts.trace = value(); break;
      case '--expect': opts.expect = value(); break;
      case '--frames': opts.frames = Number(value()); break;
      case '--bench': opts.bench = true; break;
      default: throw new Usage(`unknown option ${arg}`);
    }
  }
  for (const required of ['wasm', 'prog', 'data', 'trace']) {
    if (!opts[required]) throw new Usage(`--${required} is required`);
  }
  if (!opts.bench && !opts.expect) throw new Usage('--expect is required without --bench');
  return opts;
}

/// Trace format v1, as specified in `crates/chill65-diff/src/trace.rs`.
///
/// Read rather than reimplemented: `x<count>` expands to that many identical
/// frames, and switches named on a line are **held** for that frame, levels
/// rather than edges. Getting either wrong would show up as a divergence and be
/// blamed on the wasm build, so both are done exactly as the Rust does them.
function parseTrace(text) {
  const frames = [];
  let sawHeader = false;

  for (const raw of text.split('\n')) {
    const line = raw.replace(/#.*$/, '').trim();
    if (line === '') continue;

    if (!sawHeader) {
      if (line !== 'chill65-trace 1') throw new Usage(`bad trace header ${JSON.stringify(line)}`);
      sawHeader = true;
      continue;
    }

    const parts = line.split(/\s+/);
    if (parts.length < 3 || parts.length > 4) throw new Usage(`bad frame line ${JSON.stringify(line)}`);
    const [dxText, dyText, switchText, repeatText] = parts;

    const dx = Number(dxText);
    const dy = Number(dyText);
    if (!Number.isInteger(dx) || !Number.isInteger(dy)) {
      throw new Usage(`bad trackball delta in ${JSON.stringify(line)}`);
    }

    // A bitmask over the seven switch ids, which is what `set_switch` wants.
    let held = 0;
    if (switchText !== '-') {
      for (const name of switchText.split(',')) {
        const id = SWITCHES.indexOf(name);
        if (id < 0) throw new Usage(`unknown switch ${JSON.stringify(name)}`);
        held |= 1 << id;
      }
    }

    let repeat = 1;
    if (repeatText !== undefined) {
      if (!repeatText.startsWith('x')) throw new Usage(`bad repeat ${JSON.stringify(repeatText)}`);
      repeat = Number(repeatText.slice(1));
      if (!Number.isInteger(repeat) || repeat < 1) throw new Usage(`bad repeat count ${repeatText}`);
    }
    for (let n = 0; n < repeat; n++) frames.push({ dx, dy, held });
  }

  if (!sawHeader) throw new Usage('the trace has no header line');
  return frames;
}

/// Instantiate the module. It needs no imports; `smoke.mjs` asserts that.
function instantiate(path) {
  return new WebAssembly.Instance(new WebAssembly.Module(readFileSync(path))).exports;
}

/// A view over linear memory, derived fresh and never kept.
///
/// Memory grows when the module allocates, which detaches every existing view.
/// Note the argument order at the call sites: the **pointer is taken first**,
/// because JavaScript evaluates arguments left to right, so a `.buffer` read
/// before the call would already be dead by the time the view is built.
const view = (wasm, ptr, len) => new Uint8Array(wasm.memory.buffer, ptr, len);

/// A wasm `i64` as the sixteen hex digits `ccnative hashes` prints.
///
/// WebAssembly's `i64` reaches JavaScript as a **signed** BigInt, so any hash
/// with its top bit set arrives negative and formats as `-55eca3062ff75e25`
/// instead of `aa135cf9d008a1db`. Roughly half of all hashes have that bit, and
/// the first 162 frames of attract mode happen not to — so this reads as a
/// divergence at frame 162 rather than as a formatting fault, which is exactly
/// how it presented.
const hex64 = (value) => BigInt.asUintN(64, value).toString(16).padStart(16, '0');

/// Stage the images and cold-boot.
function boot(wasm, prog, data, dispatch) {
  if (prog.length !== wasm.prog_len()) {
    throw new Usage(`program image is ${prog.length} bytes, expected ${wasm.prog_len()}`);
  }
  if (data.length !== wasm.data_len()) {
    throw new Usage(`data image is ${data.length} bytes, expected ${wasm.data_len()}`);
  }
  view(wasm, wasm.prog_ptr(), prog.length).set(prog);
  view(wasm, wasm.data_ptr(), data.length).set(data);
  if (wasm.boot() !== 0) throw new Usage('the module refused the staged images');
  wasm.set_dispatch(dispatch ? 1 : 0);
}

/// Play `frames` of a trace, returning one 16-hex-digit hash per frame.
function play(wasm, trace, frames, collect) {
  const hashes = collect ? new Array(frames) : null;
  for (let k = 0; k < frames; k++) {
    // Past the end of a short trace the machine sits idle, matching the Rust.
    const input = trace[k] ?? { dx: 0, dy: 0, held: 0 };
    wasm.add_trackball_delta(input.dx, input.dy);
    // Every switch, every frame: they are levels, so a release has to be sent
    // as explicitly as a press.
    for (let id = 0; id < SWITCHES.length; id++) {
      wasm.set_switch(id, (input.held >> id) & 1);
    }
    if (wasm.run_frame() !== 0) throw new Usage(`the CPU faulted on frame ${k}`);
    if (collect) hashes[k] = hex64(wasm.frame_hash());
  }
  return hashes;
}

function compare(opts) {
  const wasm = instantiate(opts.wasm);
  const prog = readFileSync(opts.prog);
  const data = readFileSync(opts.data);
  const trace = parseTrace(readFileSync(opts.trace, 'utf8'));
  const expected = readFileSync(opts.expect, 'utf8').split('\n').filter((l) => l !== '');
  const frames = opts.frames ?? Math.min(trace.length, expected.length);

  if (expected.length < frames) {
    throw new Usage(`${opts.expect} has ${expected.length} hashes, need ${frames}`);
  }

  let ok = true;
  for (const [label, dispatch] of [['interpreted', false], ['dispatched', true]]) {
    boot(wasm, prog, data, dispatch);
    const got = play(wasm, trace, frames, true);

    const at = got.findIndex((h, k) => h !== expected[k]);
    if (at < 0) {
      console.log(`  ${label.padEnd(12)} ${frames} frames identical to native`);
      continue;
    }
    ok = false;
    console.log(
      `  ${label.padEnd(12)} DIVERGED at frame ${at}\n` +
      `    native ${expected[at]}\n` +
      `    wasm   ${got[at]}`,
    );
  }
  return ok;
}

function bench(opts) {
  const wasm = instantiate(opts.wasm);
  const prog = readFileSync(opts.prog);
  const data = readFileSync(opts.data);
  const trace = parseTrace(readFileSync(opts.trace, 'utf8'));
  const frames = opts.frames ?? trace.length;

  for (const [label, dispatch] of [['interpreted', false], ['dispatched', true]]) {
    boot(wasm, prog, data, dispatch);
    const start = performance.now();
    play(wasm, trace, frames, false);
    const ms = performance.now() - start;
    console.log(
      `  ${label.padEnd(12)} ${String(frames).padStart(5)} frames  ` +
      `${ms.toFixed(1).padStart(9)} ms  ${((frames * 1000) / ms).toFixed(1).padStart(8)} frames/sec`,
    );
  }
  return true;
}

try {
  const opts = parseArgs(process.argv.slice(2));
  console.log(`${opts.trace}${opts.bench ? ' (bench)' : ''}`);
  process.exit((opts.bench ? bench(opts) : compare(opts)) ? 0 : 1);
} catch (e) {
  console.error(`harness: ${e.message}`);
  process.exit(2);
}
