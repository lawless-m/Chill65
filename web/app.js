// The browser front end: canvas, mouse-as-trackball, keyboard switches.
//
// Everything the module needs is passed in as bytes — it has no filesystem —
// and everything it produces is read straight out of its linear memory. There
// is no generated glue: `chill65-wasm` exports a plain C ABI and this is the
// whole of the other side of it.
//
// # The parts that are easy to get wrong
//
// **Views die when memory grows.** Every `Uint8Array` over `exports.memory
// .buffer` is invalidated the moment the module allocates, and the module
// allocates a framebuffer every time it renders. So views are derived at the
// point of use and never stored — and the pointer is always taken *before*
// `.buffer` is read, because JavaScript evaluates arguments left to right and
// would otherwise capture a buffer the call is about to detach.
//
// **A display frame is not a game frame.** The board runs at 61.035 Hz
// (`frame.rs`: 1,250,000 CPU cycles per second ÷ 20,480 per frame) and a
// monitor runs at whatever it runs at. Tying one to the other would play the
// game fast on a 120 Hz panel and slow on a 50 Hz one, so game frames are
// driven from accumulated real time instead.
//
// **Switches are levels, not edges.** `keydown` and `keyup` set and clear them;
// the machine goes on seeing a button held until told otherwise.

const FRAME_HZ = 1250000 / 20480; // 61.035, from frame.rs
const FRAME_MS = 1000 / FRAME_HZ;

// Guard against the tab having been in the background: catching up on ten
// minutes of missed frames would hang the page for a minute.
const MAX_CATCHUP_FRAMES = 4;

// --- sound ------------------------------------------------------------------
//
// The module hands us the hardware's own samples: one byte per CPU cycle at
// 1.25 MHz, the sum of two six-bit POKEYs, so 0-120. Nothing resamples them for
// us, because the module has no audio device — the same seam the EAROM sits on.
//
// The hard part is not making a noise, it is not making a bad one. Game frames
// are driven from accumulated real time in a `requestAnimationFrame` loop, and
// that clock has nothing to do with the audio device's. So each frame's samples
// are scheduled against the AudioContext's own clock, at a cursor that runs
// ahead of it.
const CPU_HZ = 1250000;
// SOUT's range: two six-bit chips summed.
const SOUT_MAX = 120;
// How far ahead of the context clock chunks are scheduled. It has to absorb
// requestAnimationFrame jitter — 16.7 ms between frames at 60 Hz, and much more
// when the browser is busy — plus a dropped frame or two, while staying short
// enough that the sound still feels attached to the picture. 80 ms is about
// five game frames.
const AUDIO_LATENCY = 0.08;
// Loud enough to hear, quiet enough not to clip when several channels sound at
// once. Applied after scaling a channel's 0-15 level by 64.
const AUDIO_GAIN = 0.6;
// One-pole coefficient for the DC estimate subtracted before playback. A
// volume-only channel is a constant offset, and feeding that to a speaker is a
// thump on every change rather than a note. ~4 Hz at 48 kHz.
const DC_POLE = 0.0008;

// Mouse pixels to trackball counts.
//
// `hardware.md` lists this as UNVERIFIED: nothing establishes how a host's
// pointer delta should map to the counts a real trackball produced, and the
// board cannot tell us. Passing movement through 1:1 — which this did — makes
// the game unplayably twitchy, so the factor is real and simply unmeasured.
//
// Adjustable at runtime with [ and ], because the honest way to fix an
// unverified constant is to let someone find it by feel and then write the
// number down. Remembered across reloads so two host devices can be compared
// without retuning from scratch each time — a mouse and a real trackball need
// not want the same number, and if they do not, the constant belongs to this
// page rather than to the board.
let trackballScale = Number(localStorage.getItem('chill65.scale')) || 0.35;

// Scaling below 1 with truncation would drop every small movement and feel
// dead, so the remainder is carried rather than discarded.
let owedX = 0;
let owedY = 0;

// Which way vertical runs, toggled with Y.
//
// Also not merely a preference. `input.rs` records that `CCastles.v:292` wires
// two trackball pairs whose roles are not established — a reversed axis would
// show as inverted motion — and `harness.md` §11.6 says the same of the
// quadrature phase. So this switch is a probe of an open question as much as a
// comfort setting.
let invertY = localStorage.getItem('chill65.invertY') === '1';

// Switch ids, in the order `chill65-wasm`'s `set_switch` numbers them.
const SWITCH = {
  Start1: 0,
  Start2: 1,
  SelfTest: 2,
  Slam: 3,
  CoinAux: 4,
  CoinLeft: 5,
  CoinRight: 6,
};

const KEYS = {
  Digit1: SWITCH.Start1,
  Digit2: SWITCH.Start2,
  Digit5: SWITCH.CoinLeft,
  Digit6: SWITCH.CoinRight,
  Digit7: SWITCH.CoinAux,
  KeyT: SWITCH.SelfTest,
  KeyS: SWITCH.Slam,
};

// The EAROM is where the board kept its high scores. The runtime models it as
// RAM — it has no storage, and on wasm32 no filesystem to have one in — so
// surviving a reload is this page's job. Saved on a timer rather than every
// frame: 256 bytes through localStorage sixty times a second would be silly,
// and the game writes the table rarely.
const EAROM_KEY = 'chill65.earom';
const EAROM_SAVE_MS = 2000;

/// Read the saved table, or null if there is none or it is the wrong size.
function loadEarom(want) {
  const text = localStorage.getItem(EAROM_KEY);
  if (!text) return null;
  try {
    const raw = atob(text);
    if (raw.length !== want) return null;
    return Uint8Array.from(raw, (c) => c.charCodeAt(0));
  } catch {
    // Corrupt or from an older build: start fresh rather than refuse to boot.
    return null;
  }
}

function saveEarom(bytes) {
  localStorage.setItem(EAROM_KEY, btoa(String.fromCharCode(...bytes)));
}

const canvas = document.getElementById('screen');
const status = document.getElementById('status');
const context = canvas.getContext('2d');

const say = (text, isError = false) => {
  status.textContent = text;
  status.classList.toggle('error', isError);
};

/// A view over the module's linear memory, derived fresh every time.
const view = (wasm, ptr, len) => new Uint8Array(wasm.memory.buffer, ptr, len);

async function fetchBytes(name) {
  const response = await fetch(name);
  if (!response.ok) throw new Error(`${name}: ${response.status} ${response.statusText}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function main() {
  // The module needs no imports at all, which is what a hand-written C ABI
  // buys: nothing here has to be kept in step with the Rust.
  const [{ instance }, prog, data, mob] = await Promise.all([
    WebAssembly.instantiateStreaming(fetch('chill65.wasm'), {}),
    fetchBytes('prog.bin'),
    fetchBytes('data.bin'),
    fetchBytes('mob.bin'),
  ]);
  const wasm = instance.exports;

  if (
    prog.length !== wasm.prog_len() ||
    data.length !== wasm.data_len() ||
    mob.length !== wasm.mob_len()
  ) {
    throw new Error(
      `image sizes are ${prog.length}/${data.length}/${mob.length}, ` +
      `the module wants ${wasm.prog_len()}/${wasm.data_len()}/${wasm.mob_len()}`,
    );
  }

  view(wasm, wasm.prog_ptr(), prog.length).set(prog);
  view(wasm, wasm.data_ptr(), data.length).set(data);
  // The motion-object picture ROMs: without these the castle draws and the
  // characters do not.
  view(wasm, wasm.mob_ptr(), mob.length).set(mob);
  if (wasm.boot() !== 0) throw new Error('the module refused the images');

  // After boot, because boot builds a fresh machine and would wipe it.
  const saved = loadEarom(wasm.earom_len());
  if (saved) {
    view(wasm, wasm.earom_ptr(), saved.length).set(saved);
  }
  let lastEarom = Array.from(view(wasm, wasm.earom_ptr(), wasm.earom_len()));
  let nextSave = 0;

  let dispatch = true;
  wasm.set_dispatch(1);

  canvas.width = wasm.fb_width();
  canvas.height = wasm.fb_height();
  const picture = context.createImageData(canvas.width, canvas.height);

  // --- sound ------------------------------------------------------------
  let audio = null; // { ctx, gain, cursor, dc }
  let muted = false;
  const pending = [];

  // Browsers will not start an AudioContext without a gesture, so this hangs
  // off the same click that captures the pointer. The module stays inert until
  // sound can actually play: `set_audio` is only ever called from here.
  const startAudio = () => {
    if (audio) {
      if (audio.ctx.state === 'suspended') audio.ctx.resume();
      return;
    }
    const Ctor = window.AudioContext ?? window.webkitAudioContext;
    if (!Ctor) return; // no Web Audio: the game still plays, silently
    const ctx = new Ctor();
    const gain = ctx.createGain();
    gain.gain.value = muted ? 0 : 1;
    gain.connect(ctx.destination);
    audio = { ctx, gain, cursor: 0, dc: SOUT_MAX / 2 };
    wasm.set_audio(1);
  };

  // Box-average the 1.25 MHz stream down to the context's rate — about 26
  // input samples per output at 48 kHz — and centre it.
  const resample = (input, rate) => {
    const count = Math.max(1, Math.round((input.length * rate) / CPU_HZ));
    const out = new Float32Array(count);
    const step = input.length / count;
    for (let j = 0; j < count; j++) {
      const from = Math.floor(j * step);
      const to = Math.min(input.length, Math.max(from + 1, Math.floor((j + 1) * step)));
      let sum = 0;
      for (let i = from; i < to; i++) sum += input[i];
      const mean = sum / (to - from);
      audio.dc += (mean - audio.dc) * DC_POLE;
      const v = ((mean - audio.dc) / 64) * AUDIO_GAIN;
      out[j] = v > 1 ? 1 : v < -1 ? -1 : v;
    }
    return out;
  };

  // Schedule everything run since the last flush as one buffer.
  const flushAudio = () => {
    if (!audio || pending.length === 0) return;
    const { ctx } = audio;
    let total = 0;
    for (const chunk of pending) total += chunk.length;
    const raw = new Uint8Array(total);
    let at = 0;
    for (const chunk of pending) {
      raw.set(chunk, at);
      at += chunk.length;
    }
    pending.length = 0;
    if (ctx.state !== 'running') return; // suspended: drop rather than pile up

    const samples = resample(raw, ctx.sampleRate);
    const buffer = ctx.createBuffer(1, samples.length, ctx.sampleRate);
    buffer.copyToChannel(samples, 0);
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(audio.gain);

    // If the cursor has fallen behind the context clock — the tab was
    // backgrounded, or MAX_CATCHUP_FRAMES dropped frames — resync it once
    // rather than let every chunk from here on start late and stutter.
    if (audio.cursor < ctx.currentTime) {
      audio.cursor = ctx.currentTime + AUDIO_LATENCY;
    }
    source.start(audio.cursor);
    audio.cursor += buffer.duration;
  };

  // --- input ------------------------------------------------------------

  canvas.addEventListener('click', () => {
    startAudio();
    canvas.requestPointerLock();
  });

  document.addEventListener('mousemove', (event) => {
    if (document.pointerLockElement !== canvas) return;
    // Positive right and down on both sides, so only the magnitude is scaled.
    owedX += event.movementX * trackballScale;
    owedY += event.movementY * trackballScale * (invertY ? -1 : 1);
    const dx = Math.trunc(owedX);
    const dy = Math.trunc(owedY);
    owedX -= dx;
    owedY -= dy;
    if (dx !== 0 || dy !== 0) {
      wasm.add_trackball_delta(dx, dy);
    }
  });

  const key = (event, held) => {
    // Trackball sensitivity, live.
    if (held && (event.code === 'BracketLeft' || event.code === 'BracketRight')) {
      const by = event.code === 'BracketRight' ? 1.25 : 0.8;
      trackballScale = Math.min(4, Math.max(0.02, trackballScale * by));
      localStorage.setItem('chill65.scale', String(trackballScale));
      event.preventDefault();
      return;
    }
    if (event.code === 'KeyY' && held) {
      invertY = !invertY;
      localStorage.setItem('chill65.invertY', invertY ? '1' : '0');
      // Drop the carried remainder rather than let it push the other way.
      owedY = 0;
      event.preventDefault();
      return;
    }
    if (event.code === 'KeyM' && held) {
      muted = !muted;
      if (audio) audio.gain.gain.value = muted ? 0 : 1;
      event.preventDefault();
      return;
    }
    if (event.code === 'KeyD' && held) {
      dispatch = !dispatch;
      wasm.set_dispatch(dispatch ? 1 : 0);
      event.preventDefault();
      return;
    }
    const id = KEYS[event.code];
    if (id === undefined) return;
    wasm.set_switch(id, held);
    event.preventDefault();
  };
  addEventListener('keydown', (event) => key(event, 1));
  addEventListener('keyup', (event) => key(event, 0));

  // --- the loop ---------------------------------------------------------

  let previous = performance.now();
  let owed = 0;
  let framesThisSecond = 0;
  let secondStarted = previous;
  let shown = 0;
  let faulted = false;

  const tick = (now) => {
    requestAnimationFrame(tick);
    if (faulted) return;

    owed += (now - previous) / FRAME_MS;
    previous = now;
    const due = Math.min(Math.floor(owed), MAX_CATCHUP_FRAMES);
    owed -= Math.floor(owed);
    if (due <= 0) return;

    for (let n = 0; n < due; n++) {
      if (wasm.run_frame() !== 0) {
        faulted = true;
        say('the CPU faulted — reload to start over', true);
        return;
      }
      framesThisSecond++;
      // Drained inside the loop: `run_frame` replaces the buffer, so catching
      // up on several frames would otherwise keep only the last one's sound.
      // Pointer before `.buffer`, and copied, because the next frame overwrites
      // it and any allocation would detach the view.
      if (audio) {
        pending.push(new Uint8Array(view(wasm, wasm.audio_ptr(), wasm.audio_len())));
      }
    }

    flushAudio();

    // Derived after `render_rgba`, which allocates: any view taken earlier
    // would be detached by now.
    const ptr = wasm.render_rgba();
    picture.data.set(view(wasm, ptr, picture.data.length));
    context.putImageData(picture, 0, 0);

    // Persist the high-score table when it changes, and not otherwise.
    if (now >= nextSave) {
      nextSave = now + EAROM_SAVE_MS;
      const current = view(wasm, wasm.earom_ptr(), wasm.earom_len());
      if (current.some((b, i) => b !== lastEarom[i])) {
        lastEarom = Array.from(current);
        saveEarom(lastEarom);
      }
    }

    if (now - secondStarted >= 1000) {
      shown = Math.round((framesThisSecond * 1000) / (now - secondStarted));
      framesThisSecond = 0;
      secondStarted = now;
    }
    say(
      `${shown} frames/sec of ${FRAME_HZ.toFixed(2)} — ` +
      `${dispatch ? 'compiled dispatch on' : 'interpreting only'} — ` +
      `trackball x${trackballScale.toFixed(2)} ([ ]), Y ${invertY ? 'inverted' : 'normal'} (Y) — ` +
      `${document.pointerLockElement === canvas ? 'captured, Esc to release' : 'click to capture'} — ` +
      `${!audio ? 'click for sound' : muted ? 'muted (M)' : 'sound on (M)'}`,
    );
  };
  requestAnimationFrame(tick);
}

main().catch((e) => {
  say(`${e.message}`, true);
  // The usual cause is opening the file directly, or staging never having run.
  console.error(e);
});
