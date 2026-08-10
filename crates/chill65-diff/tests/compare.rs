//! `ccdiff compare` end to end: a stream against an exact copy, and against a
//! copy with one line altered.
//!
//! No corpus and no environment needed — the streams are written by the test.

use std::path::PathBuf;
use std::process::Command;

use chill65_diff::diverge::write_stream;
use chill65_diff::images::workspace_root;

/// A scratch directory under `target/`, which is gitignored.
fn scratch(name: &str) -> PathBuf {
    let dir = workspace_root().join("target/boot-artefacts/compare-test").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Run `ccdiff` and return `(exit code, stdout)`.
fn ccdiff(args: &[&str]) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ccdiff"))
        .args(args)
        .output()
        .expect("run ccdiff");
    (
        out.status.code().expect("exit code"),
        String::from_utf8(out.stdout).expect("utf-8 stdout"),
    )
}

#[test]
fn compare_reports_identity_and_divergence() {
    let dir = scratch("identity");
    let hashes: Vec<u64> = (0..64).map(|i| 0x1000_0000_0000_0000 + i).collect();

    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    write_stream(&a, &hashes).expect("write a");
    std::fs::copy(&a, &b).expect("exact copy");

    let (code, stdout) = ccdiff(&["compare", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert_eq!(code, 0, "identical streams must exit 0");
    assert_eq!(stdout.trim(), "identical over 64 frames");

    // Alter exactly one line and it must be named.
    let mut altered = hashes.clone();
    altered[42] ^= 1;
    let c = dir.join("c.txt");
    write_stream(&c, &altered).expect("write c");

    let (code, stdout) = ccdiff(&["compare", a.to_str().unwrap(), c.to_str().unwrap()]);
    assert_eq!(code, 0, "a divergence is a result, not an error");
    assert_eq!(stdout.trim(), "diverged at frame 42");
}

#[test]
fn compare_states_length_mismatches() {
    let dir = scratch("lengths");
    let a = dir.join("a.txt");
    let b = dir.join("b.txt");
    write_stream(&a, &[1, 2, 3]).expect("write a");
    write_stream(&b, &[1, 2]).expect("write b");

    let (code, stdout) = ccdiff(&["compare", a.to_str().unwrap(), b.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(
        stdout.trim(),
        "identical over 2 frames (lengths differ: 3 and 2)",
        "a truncated stream must never read as a clean match"
    );
}

#[test]
fn errors_exit_nonzero() {
    let dir = scratch("errors");
    let missing = dir.join("nope.txt");
    let (code, _) = ccdiff(&["compare", missing.to_str().unwrap(), missing.to_str().unwrap()]);
    assert_ne!(code, 0, "a missing stream file is an error");

    let junk = dir.join("junk.txt");
    std::fs::write(&junk, "not a hash\n").expect("write junk");
    let (code, _) = ccdiff(&["compare", junk.to_str().unwrap(), junk.to_str().unwrap()]);
    assert_ne!(code, 0, "an unparseable stream is an error");

    let (code, _) = ccdiff(&["compare", junk.to_str().unwrap()]);
    assert_ne!(code, 0, "compare needs two arguments");
}

#[test]
fn help_exits_zero() {
    let (code, stdout) = ccdiff(&["--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("usage: ccdiff"), "{stdout}");
    assert!(stdout.contains("compare"), "{stdout}");
}
