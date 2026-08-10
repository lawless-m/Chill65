//! `ccrun` — headless runner for the Crystal Castles machine.
//!
//! ```text
//! ccrun <prog.bin> <data.bin> [--frames N] [--dump out.ppm] [--quiet]
//! ```
//!
//! Runs the machine for `N` frames and prints the frame hash. With `--dump` it
//! also writes the visible picture as a binary PPM (P6) — the human-inspectable
//! artefact for the later, human-judged "playable" milestone. PPM is used
//! precisely because it is trivial to write by hand: this crate takes no
//! third-party dependencies.
//!
//! No window and no audio, by design.

use std::io::Write;
use std::process::ExitCode;

use chill65_runtime::frame::run_frame;
use chill65_runtime::video::{cram_rgb, BITMAP_CRAM_BASE, HEIGHT, WIDTH};
use chill65_runtime::{Cpu, Machine};

const USAGE: &str = "\
usage: ccrun <prog.bin> <data.bin> [options]

  <prog.bin>       24576-byte program image (A000-FFFF)
  <data.bin>       16384-byte castle data image (A000-DFFF)
  [mob.bin]        optional 16384-byte motion-object picture ROMs
                   (136022-106.8d then 136022-107.8b); without it no
                   motion objects are drawn

options:
  --frames N       frames to run (default 600)
  --dump FILE      write the visible picture as a binary PPM
  --quiet          print only the frame hash
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ccrun: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut positional: Vec<String> = Vec::new();
    let mut frames: u32 = 600;
    let mut dump: Option<String> = None;
    let mut quiet = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "--frames" => {
                let v = args.next().ok_or("--frames needs a value")?;
                frames = v.parse().map_err(|_| format!("bad frame count {v:?}"))?;
            }
            "--dump" => dump = Some(args.next().ok_or("--dump needs a path")?),
            "--quiet" => quiet = true,
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other => positional.push(other.to_string()),
        }
    }

    if positional.len() < 2 || positional.len() > 3 {
        eprint!("{USAGE}");
        return Err("expected two image paths, or three with the motion ROMs".into());
    }

    let prog = std::fs::read(&positional[0]).map_err(|e| format!("{}: {e}", positional[0]))?;
    let data = std::fs::read(&positional[1]).map_err(|e| format!("{}: {e}", positional[1]))?;

    let mut machine = Machine::new();
    machine.load_roms(&prog, &data)?;

    // Optional: without it the machine runs exactly as it always has, drawing
    // the bitmap and no motion objects.
    if let Some(path) = positional.get(2) {
        let mob = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        machine.load_motion_roms(&mob)?;
    }

    let mut cpu = Cpu::new();
    cpu.reset(&mut machine);

    for frame in 0..frames {
        run_frame(&mut cpu, &mut machine)
            .map_err(|e| format!("{e} at frame {frame}, PC {:04X}", cpu.pc))?;
    }

    let picture = machine.framebuffer();
    let hash = machine.frame_hash();

    if let Some(path) = &dump {
        write_ppm(path, &picture, &machine.video.cram)?;
    }

    if quiet {
        println!("{hash:016x}");
    } else {
        let lit = picture.iter().filter(|&&p| p != 0).count();
        println!(
            "frames {frames}, cycles {}, hash {hash:016x}\n\
             lit pixels {lit}/{}, watchdog strobes {}{}, bank switches {}",
            machine.cycles,
            picture.len(),
            machine.watchdog_strobes,
            if machine.watchdog_expired {
                " (EXPIRED)"
            } else {
                ""
            },
            machine.bank_switches,
        );
        if let Some(path) = &dump {
            println!("wrote {WIDTH}x{HEIGHT} PPM to {path}");
        }
    }
    Ok(())
}

/// Write the picture as a binary PPM, applying colour RAM.
fn write_ppm(path: &str, picture: &[u8], cram: &[u16; 32]) -> Result<(), String> {
    let mut out = Vec::with_capacity(15 + WIDTH * HEIGHT * 3);
    out.extend_from_slice(format!("P6\n{WIDTH} {HEIGHT}\n255\n").as_bytes());
    for &pixel in picture {
        let entry = cram[(BITMAP_CRAM_BASE + pixel as usize) & 0x1F];
        let (r, g, b) = cram_rgb(entry);
        out.extend_from_slice(&[r, g, b]);
    }
    let mut f = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    f.write_all(&out).map_err(|e| format!("{path}: {e}"))?;
    Ok(())
}
