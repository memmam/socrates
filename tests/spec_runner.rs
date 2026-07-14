//! Golden test runner: executes every `tests/spec/**/*.fable` program and
//! checks it against the directives embedded in its comments.
//!
//! Directives (each on its own line, anywhere in the file):
//!   //? expect: <line>     — expected stdout, one directive per line, in order.
//!   //? error: <substring> — compilation must fail; some diagnostic
//!                            (message or code) must contain <substring>.
//!   //? panic: <substring> — the program must panic at runtime; the panic
//!                            message must contain <substring>.
//!
//! A file with no directives must merely run to completion with no output.
//! `expect` cannot be combined with `error`; `expect` + `panic` means the
//! output-so-far must match before the panic.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use fable::RunOutcome;

#[derive(Default)]
struct Directives {
    expect: Vec<String>,
    errors: Vec<String>,
    panics: Vec<String>,
}

fn parse_directives(text: &str) -> Directives {
    let mut d = Directives::default();
    for line in text.lines() {
        let Some(idx) = line.find("//?") else { continue };
        let rest = line[idx + 3..].trim_start();
        if let Some(v) = rest.strip_prefix("expect:") {
            d.expect.push(v.strip_prefix(' ').unwrap_or(v).to_string());
        } else if let Some(v) = rest.strip_prefix("error:") {
            d.errors.push(v.trim().to_string());
        } else if let Some(v) = rest.strip_prefix("panic:") {
            d.panics.push(v.trim().to_string());
        }
    }
    d
}

fn collect_fable_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            collect_fable_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "fable") {
            out.push(p);
        }
    }
}

fn check_one(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read failed: {e}"))?;
    let d = parse_directives(&text);
    // Path-based so `import` resolves sibling files; single-file behavior is
    // identical to the old text-based runner.
    let outcome = fable::run_capture_path(path);

    match outcome {
        RunOutcome::CompileError(diags) => {
            if d.errors.is_empty() {
                let mut msg = String::from("unexpected compile error(s):\n");
                for di in diags.iter().filter(|di| di.is_error()) {
                    let _ = writeln!(msg, "  [{}] {}", di.code, di.message);
                }
                return Err(msg);
            }
            for want in &d.errors {
                let hit = diags.iter().any(|di| {
                    di.is_error() && (di.message.contains(want) || di.code.contains(want))
                });
                if !hit {
                    let mut msg =
                        format!("expected a compile error containing {want:?}; got:\n");
                    for di in diags.iter().filter(|di| di.is_error()) {
                        let _ = writeln!(msg, "  [{}] {}", di.code, di.message);
                    }
                    return Err(msg);
                }
            }
            Ok(())
        }
        RunOutcome::Panic { stdout, error } => {
            if d.panics.is_empty() {
                return Err(format!(
                    "unexpected runtime panic: {}\n--- stdout so far ---\n{stdout}",
                    error.msg
                ));
            }
            for want in &d.panics {
                if !error.msg.contains(want) {
                    return Err(format!(
                        "expected panic containing {want:?}, got: {}",
                        error.msg
                    ));
                }
            }
            check_stdout(&d.expect, &stdout)
        }
        RunOutcome::Ok { stdout, .. } => {
            if !d.errors.is_empty() {
                return Err(format!(
                    "expected a compile error containing {:?}, but the program compiled",
                    d.errors
                ));
            }
            if !d.panics.is_empty() {
                return Err(format!(
                    "expected a panic containing {:?}, but the program succeeded",
                    d.panics
                ));
            }
            check_stdout(&d.expect, &stdout)
        }
    }
}

fn check_stdout(expect: &[String], stdout: &str) -> Result<(), String> {
    let got: Vec<&str> = stdout.lines().collect();
    if got.len() != expect.len()
        || !got.iter().zip(expect).all(|(g, e)| *g == e.as_str())
    {
        let mut msg = String::new();
        let _ = writeln!(msg, "output mismatch");
        let _ = writeln!(msg, "--- expected ({} lines) ---", expect.len());
        for e in expect {
            let _ = writeln!(msg, "{e}");
        }
        let _ = writeln!(msg, "--- actual ({} lines) ---", got.len());
        for g in &got {
            let _ = writeln!(msg, "{g}");
        }
        return Err(msg);
    }
    Ok(())
}

#[test]
fn spec_suite() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/spec");
    let mut files = Vec::new();
    collect_fable_files(&root, &mut files);
    assert!(!files.is_empty(), "no spec tests found under {}", root.display());

    let mut failures = Vec::new();
    for f in &files {
        if let Err(msg) = check_one(f) {
            let rel = f.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(f);
            failures.push(format!("=== {} ===\n{msg}", rel.display()));
        }
    }
    if !failures.is_empty() {
        panic!(
            "{} of {} spec tests failed:\n\n{}",
            failures.len(),
            files.len(),
            failures.join("\n")
        );
    }
    eprintln!("spec suite: {} tests passed", files.len());
}
