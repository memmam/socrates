//! The interpreter's own golden spec suite, running through the same
//! library code as the user-facing `socrates test` command (src/testing.rs).

use std::path::Path;

#[test]
fn spec_suite() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/spec");
    let report = socrates::testing::run_test_paths(std::slice::from_ref(&root));
    assert!(report.total > 0, "no spec tests found under {}", root.display());
    if !report.failures.is_empty() {
        let mut msg = String::new();
        for (path, why) in &report.failures {
            let rel = path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(path);
            msg.push_str(&format!("=== {} ===\n{why}\n", rel.display()));
        }
        panic!(
            "{} of {} spec tests failed:\n\n{msg}",
            report.failures.len(),
            report.total
        );
    }
    eprintln!("spec suite: {} tests passed", report.total);
}
