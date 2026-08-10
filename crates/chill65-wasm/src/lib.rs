//! The Crystal Castles machine as a WebAssembly module.
//!
//! # The interface is hand-written
//!
//! `wasm-bindgen` is installed on this machine and is deliberately not used.
//! The workspace takes no third-party crates, and that posture is not suspended
//! for the last crate in it. What is exported instead is a plain C ABI —
//! integers, and pointers into the module's own linear memory — which every
//! WebAssembly embedder can call with no toolchain at all. The JavaScript side
//! is a `WebAssembly.instantiate` and a `Uint8Array` view over the exported
//! memory; there is no generated glue to keep in step.
//!
//! # Why the ROM images are handed in
//!
//! `wasm32-unknown-unknown` has no operating system: no files, no processes, no
//! clock. `chill65-diff` builds the images by *running the assembler as a
//! subprocess*, which cannot happen here. So the images arrive as bytes: JS
//! writes them into the staging buffers [`prog_ptr`] and [`data_ptr`] point at,
//! then calls [`boot`].
//!
//! That is also the distribution posture holding (plan §9). The module contains
//! the machine and, if built with a corpus, compiled routines; the ROM images
//! are supplied by whoever runs it, exactly as `README.md` requires.
//!
//! # One machine, global
//!
//! WebAssembly is single-threaded and this module is one machine, so the state
//! is a single global rather than a handle passed across the boundary. Every
//! accessor below is the only writer at any moment — there is no concurrency to
//! be wrong about.

// Imported as a module so the export below can carry the name `run_frame`
// without colliding with the runtime's own.
use chill65_native::Registry;
use chill65_runtime::frame;
use chill65_runtime::video::{cram_rgb, HEIGHT, WIDTH};
use chill65_runtime::{Cpu, Machine, Switch};

const PROG_LEN: usize = 24576;
const DATA_LEN: usize = 16384;
/// The motion-object picture ROMs: 136022-106.8d then 136022-107.8b.
const MOB_LEN: usize = 16384;
const RGBA_LEN: usize = WIDTH * HEIGHT * 4;

/// Everything that survives between calls.
struct State {
    prog: [u8; PROG_LEN],
    data: [u8; DATA_LEN],
    mob: [u8; MOB_LEN],
    rgba: [u8; RGBA_LEN],
    machine: Machine,
    cpu: Cpu,
    /// Built once and kept: `Registry::game()` accumulates the per-routine
    /// counts, and rebuilding it every frame would throw them away.
    registry: Registry,
    dispatch: bool,
}

static mut STATE: Option<State> = None;

/// The single global machine.
///
/// # Safety
///
/// WebAssembly is single-threaded and every export below runs to completion
/// before the next can start, so no two borrows exist at once.
#[allow(static_mut_refs)]
fn state() -> &'static mut State {
    unsafe {
        if STATE.is_none() {
            STATE = Some(State {
                prog: [0; PROG_LEN],
                data: [0; DATA_LEN],
                mob: [0; MOB_LEN],
                rgba: [0; RGBA_LEN],
                machine: Machine::new(),
                cpu: Cpu::new(),
                registry: Registry::game(),
                dispatch: false,
            });
        }
        STATE.as_mut().expect("just initialised")
    }
}

/// Where JavaScript writes the 24576-byte program image.
#[no_mangle]
pub extern "C" fn prog_ptr() -> *mut u8 {
    state().prog.as_mut_ptr()
}

/// Where JavaScript writes the 16384-byte castle-data image.
#[no_mangle]
pub extern "C" fn data_ptr() -> *mut u8 {
    state().data.as_mut_ptr()
}

/// Where JavaScript writes the 16384-byte motion-object picture ROMs.
///
/// Optional in one specific sense: an **all-zero** staging buffer is treated as
/// "not supplied" and the machine runs without sprites. That is what lets the
/// corpus-free host test — which stages the committed fixture program and
/// nothing else — keep working, and an all-zero picture ROM would draw nothing
/// anyway, since colour 0 is opaque black rather than transparent.
#[no_mangle]
pub extern "C" fn mob_ptr() -> *mut u8 {
    state().mob.as_mut_ptr()
}

#[no_mangle]
pub extern "C" fn mob_len() -> u32 {
    MOB_LEN as u32
}

#[no_mangle]
pub extern "C" fn prog_len() -> u32 {
    PROG_LEN as u32
}

#[no_mangle]
pub extern "C" fn data_len() -> u32 {
    DATA_LEN as u32
}

/// Cold-boot the machine from the staged images. 0 on success, 1 on failure.
///
/// Called again, it starts over from cold — which is what makes a run
/// repeatable, and what the determinism test relies on.
#[no_mangle]
pub extern "C" fn boot() -> u32 {
    let s = state();
    let mut machine = Machine::new();
    // Copied out first: `load_roms` borrows the images while `machine` is
    // borrowed mutably, and both live in `s`.
    let prog = s.prog;
    let data = s.data;
    if machine.load_roms(&prog, &data).is_err() {
        return 1;
    }
    // All zeros means nobody staged them; see `mob_ptr`.
    let mob = s.mob;
    if mob.iter().any(|&b| b != 0) && machine.load_motion_roms(&mob).is_err() {
        return 1;
    }
    let mut cpu = Cpu::new();
    cpu.reset(&mut machine);
    s.machine = machine;
    s.cpu = cpu;
    s.registry = Registry::game();
    0
}

/// Run compiled routines, or interpret everything.
#[no_mangle]
pub extern "C" fn set_dispatch(on: u32) {
    state().dispatch = on != 0;
}

/// Addresses the dispatch can enter compiled code at. Zero without a corpus.
///
/// Not a routine count: a state-machine routine registers **every block** as an
/// entry, so that the dispatch can resume it rather than only start it. Twenty
/// or so routines come to a few hundred entries.
#[no_mangle]
pub extern "C" fn dispatch_entries() -> u32 {
    state().registry.len() as u32
}

/// Trackball movement, accumulated until the frame boundary latches it.
#[no_mangle]
pub extern "C" fn add_trackball_delta(dx: i32, dy: i32) {
    state().machine.input.add_trackball_delta(dx, dy);
}

/// A switch, by the index the trace format serialises them in.
///
/// Switches are **levels**, not edges: a release must be sent as explicitly as
/// a press, or the machine goes on seeing the button held.
#[no_mangle]
pub extern "C" fn set_switch(id: u32, held: u32) {
    let switch = match id {
        0 => Switch::Start1,
        1 => Switch::Start2,
        2 => Switch::SelfTest,
        3 => Switch::Slam,
        4 => Switch::CoinAux,
        5 => Switch::CoinLeft,
        6 => Switch::CoinRight,
        _ => return,
    };
    state().machine.input.set_switch(switch, held != 0);
}

/// One frame. 0 on success, 1 if the CPU faulted.
#[no_mangle]
pub extern "C" fn run_frame() -> u32 {
    let s = state();
    let result = if s.dispatch {
        frame::run_frame_with(&mut s.cpu, &mut s.machine, &mut s.registry)
    } else {
        frame::run_frame(&mut s.cpu, &mut s.machine)
    };
    u32::from(result.is_err())
}

/// The frame hash — colour RAM addresses, not colours.
///
/// This is the number the native build's `ccnative hashes` prints, and the one
/// the two targets are compared on — now over colour RAM addresses, so motion
/// objects are inside it. It reaches JavaScript as a `BigInt`.
#[no_mangle]
pub extern "C" fn frame_hash() -> u64 {
    state().machine.frame_hash()
}

#[no_mangle]
pub extern "C" fn cycles() -> u64 {
    state().machine.cycles
}

#[no_mangle]
pub extern "C" fn fb_width() -> u32 {
    WIDTH as u32
}

#[no_mangle]
pub extern "C" fn fb_height() -> u32 {
    HEIGHT as u32
}

/// Fill and return the RGBA buffer, ready for `putImageData`.
///
/// Colour RAM is applied exactly as `ccrun`'s `write_ppm` does — the same
/// entry, the same `cram_rgb` — so the picture in a browser is the picture in a
/// PPM, with an alpha byte added. `Machine::framebuffer` already arbitrated
/// bitmap against motion objects, so the byte *is* the colour RAM address and
/// `BITMAP_CRAM_BASE` must not be added again.
#[no_mangle]
pub extern "C" fn render_rgba() -> *const u8 {
    let s = state();
    let picture = s.machine.framebuffer();
    let cram = &s.machine.video.cram;
    for (out, &pixel) in s.rgba.chunks_exact_mut(4).zip(picture.iter()) {
        // The framebuffer already carries the arbitrated colour RAM address.
        let entry = cram[pixel as usize & 0x1F];
        let (r, g, b) = cram_rgb(entry);
        out.copy_from_slice(&[r, g, b, 255]);
    }
    s.rgba.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test, because the state is one global.
    ///
    /// It runs on the host, with no corpus: the images staged are
    /// `chill65-native`'s committed fixture program and a zeroed data image, so
    /// this exercises the whole boundary — staging, boot, frames, hashing —
    /// with nothing game-derived in sight.
    #[test]
    fn the_module_boots_runs_and_repeats_itself() {
        unsafe {
            let prog = std::slice::from_raw_parts_mut(prog_ptr(), prog_len() as usize);
            prog.fill(0);
            prog[..chill65_native::fixture::IMAGE.len()]
                .copy_from_slice(chill65_native::fixture::IMAGE);
            std::slice::from_raw_parts_mut(data_ptr(), data_len() as usize).fill(0);
        }

        assert_eq!(boot(), 0, "the fixture image should load");
        for _ in 0..60 {
            assert_eq!(run_frame(), 0);
        }
        assert!(cycles() > 0, "60 frames should have cost something");
        let first = frame_hash();
        let cycles_first = cycles();

        // A second cold boot must reproduce the first exactly. Without that,
        // nothing downstream — comparing against the native hash stream, or
        // against another run of this same module — means anything.
        assert_eq!(boot(), 0);
        for _ in 0..60 {
            assert_eq!(run_frame(), 0);
        }
        assert_eq!(frame_hash(), first, "the module must be deterministic");
        assert_eq!(cycles(), cycles_first);

        // And the picture is the size the canvas expects.
        let rgba = unsafe {
            std::slice::from_raw_parts(render_rgba(), (fb_width() * fb_height() * 4) as usize)
        };
        assert_eq!(rgba.len(), 256 * 232 * 4);
        assert!(rgba.chunks_exact(4).all(|p| p[3] == 255), "opaque");
    }
}
