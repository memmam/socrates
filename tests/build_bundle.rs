//! End-to-end tests for `socrates build`: a stapled binary must run its embedded
//! program and produce byte-for-byte the same output as running that program
//! from source with `socrates demos/<name>/main.soc`. Covers the paths most
//! likely to break under extraction: imported modules, a bundled data file
//! read at runtime, and worker isolates spawned from a separate `.soc`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn socrates() -> &'static str {
    env!("CARGO_BIN_EXE_socrates")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// `socrates demos/<demo>/main.soc`, from the repo root — the canonical run a
/// stapled binary must reproduce.
fn run_from_source(demo: &str) -> Vec<u8> {
    let out = Command::new(socrates())
        .current_dir(repo_root())
        .arg(format!("demos/{demo}/main.soc"))
        .output()
        .expect("run demo from source");
    assert!(out.status.success(), "source run of {demo} failed:\n{}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

/// Build `demos/<demo>` into a self-contained binary under `tmp`, then run it
/// from an unrelated working directory and return its stdout.
fn build_and_run(demo: &str, tmp: &Path) -> Vec<u8> {
    let bin = tmp.join(demo);
    let built = Command::new(socrates())
        .current_dir(repo_root())
        .arg("build")
        .arg(format!("demos/{demo}"))
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("socrates build");
    assert!(built.status.success(), "build of {demo} failed:\n{}", String::from_utf8_lossy(&built.stderr));
    assert!(bin.exists(), "build reported success but produced no binary");

    let run = Command::new(&bin)
        .current_dir(tmp) // a cwd with none of the program's files in sight
        .output()
        .expect("run stapled binary");
    assert!(run.status.success(), "stapled {demo} exited nonzero:\n{}", String::from_utf8_lossy(&run.stderr));
    run.stdout
}

fn check(demo: &str) {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("zoo-{demo}"));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let from_source = run_from_source(demo);
    let from_binary = build_and_run(demo, &tmp);
    assert_eq!(
        String::from_utf8_lossy(&from_source),
        String::from_utf8_lossy(&from_binary),
        "stapled `{demo}` output diverged from its source run",
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Imported modules + a bundled data file (`cities.csv`) read at runtime.
#[test]
fn csvql_binary_matches_source() {
    check("csvql");
}

/// An artifact producer: writes `out.png`, reads it back, and compares
/// against the committed copy — all inside the extracted scratch tree.
#[test]
fn png_binary_matches_source() {
    check("png");
}

/// Worker isolates spawned from a separate `.soc` file, resolved against
/// the extracted program directory.
#[test]
fn parmandel_binary_matches_source() {
    check("parmandel");
}
