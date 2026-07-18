//! The module search path: imports resolve file-relative first, then against
//! the provided search directories (`SOCRATES_PATH` in the CLI).

use std::path::{Path, PathBuf};

use socrates::RunOutcome;

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(rel)
}

#[test]
fn import_resolves_via_search_path() {
    let search = vec![fixture("shared_lib")];
    match socrates::run_capture_path_with(&fixture("app/main.soc"), &search) {
        RunOutcome::Ok { stdout, .. } => {
            assert_eq!(stdout, "HELLO!\nfab\n");
        }
        RunOutcome::CompileError(diags) => {
            panic!("unexpected compile errors: {:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>())
        }
        RunOutcome::Panic { error, .. } => panic!("unexpected panic: {}", error.msg),
    }
}

#[test]
fn missing_module_reports_all_locations_tried() {
    let search = vec![fixture("shared_lib")];
    match socrates::run_capture_path_with(&fixture("app/broken.soc"), &search) {
        RunOutcome::CompileError(diags) => {
            let d = diags.iter().find(|d| d.code == "E0337").expect("an E0337 diagnostic");
            assert!(d.message.contains("no_such_thing"), "message: {}", d.message);
            // The SOCRATES_PATH candidate appears as a note.
            assert!(
                d.notes.iter().any(|n| n.contains("shared_lib") && n.contains("SOCRATES_PATH")),
                "notes: {:?}",
                d.notes
            );
        }
        _ => panic!("expected a compile error"),
    }
}

#[test]
fn file_relative_wins_over_search_path() {
    // app/ contains its own textutil.soc? No — so add a sibling with the
    // same name in a different fixture to prove precedence.
    let search = vec![fixture("shared_lib")];
    match socrates::run_capture_path_with(&fixture("app_shadow/main.soc"), &search) {
        RunOutcome::Ok { stdout, .. } => {
            assert_eq!(stdout, "local textutil\n");
        }
        RunOutcome::CompileError(diags) => {
            panic!("unexpected compile errors: {:?}", diags.iter().map(|d| &d.message).collect::<Vec<_>>())
        }
        RunOutcome::Panic { error, .. } => panic!("unexpected panic: {}", error.msg),
    }
}
