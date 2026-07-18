//! REPL import support (v0.5): std and file imports resolve interactively,
//! persist across chunks, and never load a module twice.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn repl_imports_std_and_files() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/spec/module_system");
    let mut child = Command::new(env!("CARGO_BIN_EXE_socrates"))
        .arg("repl")
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn repl");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(
            b"import std.iter;\n\
              iter.count_from(5).take(3).collect()\n\
              import geo;\n\
              let p = geo.Point { x: 3.0, y: 4.0 };\n\
              p.dist(geo.origin())\n\
              geo.bump()\n\
              import geo;\n\
              geo.bump()\n",
        )
        .unwrap();
    let out = child.wait_with_output().expect("repl output");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[5, 6, 7] : List[Int]"), "stdout: {stdout}");
    assert!(stdout.contains("5.0 : Float"), "stdout: {stdout}");
    // bump() twice across a re-import: module state persisted (1 then 2, not 1 twice).
    assert!(stdout.contains("1 : Int"), "stdout: {stdout}");
    assert!(stdout.contains("2 : Int"), "stdout: {stdout}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("error["), "stderr: {stderr}");
}
