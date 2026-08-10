// Does the WebAssembly module load and run at all?
//
// Node, not a browser. A browser build gated on a human looking at a canvas is
// not something an unattended run can finish; Node has WebAssembly built in, so
// everything up to the pixels can be checked by a machine. What a human still
// has to judge is the browser page, and that is said plainly rather than
// claimed.
//
// No npm packages. `node:fs`, `node:assert` and `node:url` ship with Node, and
// `WebAssembly` is the language. Same posture as the Rust side: nothing
// third-party, all the way down.
//
//   node web/smoke.mjs [path/to/chill65_wasm.wasm]
//
// The module is exercised with the staging buffers left **zeroed** — no game
// bytes at all. A 6502 whose every byte is zero reads a reset vector of 0000
// and executes BRK forever, which is a perfectly good frame to run: it proves
// the machine boots, ticks and hashes without needing anything undistributable
// present. Whether it produces the *right* picture is a different question, and
// `harness.mjs` asks it against the native hash stream.

import { readFileSync } from 'node:fs';
import { strict as assert } from 'node:assert';
import { fileURLToPath } from 'node:url';

const FRAMES = 60;
const WIDTH = 256;
const HEIGHT = 232;

const DEFAULT_WASM = fileURLToPath(
  new URL('../target/wasm32-unknown-unknown/release/chill65_wasm.wasm', import.meta.url),
);

/// Every export `chill65-wasm` promises, plus the memory the C ABI points into.
const REQUIRED = [
  'memory',
  'prog_ptr',
  'data_ptr',
  'boot',
  'set_dispatch',
  'add_trackball_delta',
  'set_switch',
  'run_frame',
  'frame_hash',
  'cycles',
  'render_rgba',
  'fb_width',
  'fb_height',
];

const path = process.argv[2] ?? DEFAULT_WASM;
const bytes = readFileSync(path);
const module = new WebAssembly.Module(bytes);

// A hand-written C ABI should need nothing from the host: no allocator hooks,
// no bindgen shims, no clock. If that ever stops being true the stubs belong
// here, with a note saying which export forced them.
const imports = WebAssembly.Module.imports(module);
assert.equal(
  imports.length,
  0,
  `the module should need no imports; it asks for ${JSON.stringify(imports)}`,
);

const wasm = new WebAssembly.Instance(module).exports;
for (const name of REQUIRED) {
  assert.equal(typeof wasm[name], name === 'memory' ? 'object' : 'function', `missing export ${name}`);
}

assert.equal(wasm.prog_len(), 24576, 'program image size');
assert.equal(wasm.data_len(), 16384, 'castle data image size');

// Views into linear memory are invalidated whenever it grows, and the module
// allocates — so a view is derived immediately before use and never kept.
//
// Sharper than it sounds: `new Uint8Array(wasm.memory.buffer, wasm.prog_ptr(),
// …)` **fails**, because JavaScript evaluates arguments left to right. The
// buffer is captured first, then `prog_ptr()` grows memory to make room for the
// machine, and the captured buffer is detached before the view is built. Take
// the pointer first, read `.buffer` second — always in that order.
const view = (ptr, len) => new Uint8Array(wasm.memory.buffer, ptr, len);
view(wasm.prog_ptr(), wasm.prog_len()).fill(0);
view(wasm.data_ptr(), wasm.data_len()).fill(0);

assert.equal(wasm.boot(), 0, 'boot rejected the staged images');
for (let frame = 0; frame < FRAMES; frame++) {
  assert.equal(wasm.run_frame(), 0, `the CPU faulted on frame ${frame}`);
}

assert.equal(wasm.fb_width(), WIDTH, 'framebuffer width');
assert.equal(wasm.fb_height(), HEIGHT, 'framebuffer height');

const rgba = view(wasm.render_rgba(), WIDTH * HEIGHT * 4);
assert.equal(rgba.length, WIDTH * HEIGHT * 4);
assert.ok(
  rgba.every((byte, i) => i % 4 !== 3 || byte === 255),
  'every pixel should be opaque',
);

const hash = wasm.frame_hash().toString(16).padStart(16, '0');
// Dispatch entries, not routines: a state-machine routine registers every
// block so the dispatch can resume it. Zero means a corpus-free build.
const entries = typeof wasm.dispatch_entries === 'function' ? wasm.dispatch_entries() : 0;
console.log(`${path}
  ${bytes.length} bytes, ${entries} dispatch entries
  ${FRAMES} frames, ${wasm.cycles()} cycles, hash ${hash}
  framebuffer ${wasm.fb_width()}x${wasm.fb_height()}, opaque`);
