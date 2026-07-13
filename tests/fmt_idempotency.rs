//! Formatter invariants, enforced over the whole example + spec corpus:
//! 1. formatting a valid program yields a valid program,
//! 2. formatting is idempotent (fmt(fmt(x)) == fmt(x)),
//! 3. the formatted program behaves identically (same stdout / outcome).

use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "fable") {
            out.push(p);
        }
    }
}

fn outcome_fingerprint(name: &str, text: &str) -> String {
    match fable::run_capture(name, text) {
        fable::RunOutcome::Ok { stdout, .. } => format!("ok:{stdout}"),
        fable::RunOutcome::Panic { stdout, error } => {
            format!("panic:{}:{stdout}", error.msg)
        }
        fable::RunOutcome::CompileError(diags) => {
            let codes: Vec<&str> =
                diags.iter().filter(|d| d.is_error()).map(|d| d.code).collect();
            format!("err:{}", codes.join(","))
        }
    }
}

#[test]
fn formatter_is_idempotent_and_behavior_preserving() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect(&root.join("examples"), &mut files);
    collect(&root.join("tests/spec"), &mut files);
    assert!(!files.is_empty());

    let mut failures = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let name = f.display().to_string();

        // Skip files that (intentionally) fail to parse — fmt refuses them.
        let Ok(once) = fable::fmt::format_source(&name, &text) else { continue };
        match fable::fmt::format_source(&name, &once) {
            Ok(twice) => {
                if once != twice {
                    failures.push(format!("{name}: not idempotent"));
                    continue;
                }
            }
            Err(_) => {
                failures.push(format!("{name}: formatted output fails to parse"));
                continue;
            }
        }

        // Behavior preservation. Examples that read stdin or are slow in
        // debug builds are exempted from execution (still checked above).
        let base = f.file_name().unwrap().to_string_lossy().to_string();
        if matches!(base.as_str(), "adventure.fable" | "raytracer.fable" | "bench.fable") {
            continue;
        }
        let before = outcome_fingerprint(&name, &text);
        let after = outcome_fingerprint(&name, &once);
        if before != after {
            failures.push(format!(
                "{name}: behavior changed after formatting\n--- before ---\n{before}\n--- after ---\n{after}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} formatter failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
