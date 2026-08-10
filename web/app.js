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

  let dispatch = true;
  wasm.set_dispatch(1);

  canvas.width = wasm.fb_width();
  canvas.height = wasm.fb_height();
  const picture = context.createImageData(canvas.width, canvas.height);

  // --- input ------------------------------------------------------------

  canvas.addEventListener('click', () => canvas.requestPointerLock());

  document.addEventListener('mousemove', (event) => {
    if (document.pointerLockElement !== canvas) return;
    // Raw movement, unscaled. The trace convention is positive right and down,
    // which is what the browser reports, so nothing is transformed here — if
    // the game proves unplayable the factor belongs at this line, named.
    wasm.add_trackball_delta(event.movementX, event.movementY);
  });

  const key = (event, held) => {
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
    }

    // Derived after `render_rgba`, which allocates: any view taken earlier
    // would be detached by now.
    const ptr = wasm.render_rgba();
    picture.data.set(view(wasm, ptr, picture.data.length));
    context.putImageData(picture, 0, 0);

    if (now - secondStarted >= 1000) {
      shown = Math.round((framesThisSecond * 1000) / (now - secondStarted));
      framesThisSecond = 0;
      secondStarted = now;
    }
    say(
      `${shown} frames/sec of ${FRAME_HZ.toFixed(2)} — ` +
      `${dispatch ? 'compiled dispatch on' : 'interpreting only'}, ` +
      `${wasm.dispatch_entries()} dispatch entries — ` +
      `${document.pointerLockElement === canvas ? 'trackball captured' : 'click to capture the trackball'}`,
    );
  };
  requestAnimationFrame(tick);
}

main().catch((e) => {
  say(`${e.message}`, true);
  // The usual cause is opening the file directly, or staging never having run.
  console.error(e);
});
