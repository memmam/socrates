//! The book is a test suite: every ```soc block in book/*.md is executed.
//!
//! Fence tags (invisible in rendered markdown) classify blocks:
//!   ```soc          — must compile and run without panicking
//!   ```soc errors   — must FAIL to compile (deliberate-error demos)
//!   ```soc panics   — must panic at runtime (deliberate-panic demos)
//!   ```soc skip     — not executed (fragments of larger programs)
//!
//! A block whose first line is `// <name>.soc` is written under that name
//! into the chapter's directory, so later blocks in the chapter can import
//! it. Blocks containing `//? ` directives run under full `socrates test`
//! semantics. All blocks run from the chapter's own temp directory.

use std::fmt::Write as _;
use std::path::PathBuf;

use socrates::RunOutcome;

struct Block {
    tag: String,
    body: String,
    index: usize,
}

fn extract_blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut lines = text.lines().peekable();
    let mut index = 0;
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        if let Some(info) = trimmed.strip_prefix("```soc") {
            let tag = info.trim().to_string();
            let mut body = String::new();
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with("```") {
                    break;
                }
                body.push_str(inner);
                body.push('\n');
            }
            blocks.push(Block { tag, body, index });
            index += 1;
        }
    }
    blocks
}

fn support_file_name(body: &str) -> Option<String> {
    let first = body.lines().next()?.trim();
    let name = first.strip_prefix("// ")?;
    if name.ends_with(".soc") && !name.contains(' ') {
        Some(name.to_string())
    } else {
        None
    }
}

#[test]
fn every_book_snippet_executes() {
    let book = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("book");
    let out_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/book-snippets");
    let mut failures = Vec::new();
    let mut total = 0usize;

    let mut chapters: Vec<PathBuf> = std::fs::read_dir(&book)
        .expect("book dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    chapters.sort();

    for chapter in chapters {
        let stem = chapter.file_stem().unwrap().to_string_lossy().to_string();
        let dir = out_root.join(&stem);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("chapter dir");
        let text = std::fs::read_to_string(&chapter).expect("read chapter");

        for block in extract_blocks(&text) {
            if block.tag == "skip" {
                continue;
            }
            total += 1;
            let file_name = support_file_name(&block.body)
                .unwrap_or_else(|| format!("snippet_{:02}.soc", block.index));
            let path = dir.join(&file_name);
            std::fs::write(&path, &block.body).expect("write snippet");
            let label = format!("{stem} block {} ({file_name})", block.index);

            // Directive-bearing blocks run under full golden-test semantics.
            if block.body.contains("//? ") {
                if let Err(why) = socrates::testing::check_one(&path) {
                    failures.push((label, why));
                }
                continue;
            }

            let outcome = run_in_dir(&path, &dir);
            let verdict = match (block.tag.as_str(), outcome) {
                ("", RunOutcome::Ok { .. }) => Ok(()),
                ("", RunOutcome::CompileError(diags)) => {
                    let mut why = String::from("failed to compile:\n");
                    for d in diags.iter().filter(|d| d.is_error()) {
                        let _ = writeln!(why, "  [{}] {}", d.code, d.message);
                    }
                    Err(why)
                }
                ("", RunOutcome::Panic { error, .. }) => {
                    Err(format!("panicked: {}", error.msg))
                }
                ("errors", RunOutcome::CompileError(_)) => Ok(()),
                ("errors", _) => Err("expected a compile error, but it compiled".into()),
                ("panics", RunOutcome::Panic { .. }) => Ok(()),
                ("panics", RunOutcome::Ok { .. }) => {
                    Err("expected a panic, but it ran to completion".into())
                }
                ("panics", RunOutcome::CompileError(diags)) => {
                    let first = diags.iter().find(|d| d.is_error());
                    Err(format!(
                        "expected a panic, but it failed to compile: {}",
                        first.map(|d| d.message.as_str()).unwrap_or("?")
                    ))
                }
                (tag, _) => Err(format!("unknown fence tag `{tag}`")),
            };
            if let Err(why) = verdict {
                failures.push((label, why));
            }
        }
    }

    assert!(total > 80, "expected to find many snippets, found {total}");
    if !failures.is_empty() {
        let mut msg = String::new();
        for (label, why) in &failures {
            let _ = writeln!(msg, "=== {label} ===\n{why}");
        }
        panic!(
            "{} of {total} book snippets failed:\n\n{msg}",
            failures.len()
        );
    }
    eprintln!("book snippets: {total} executed");
}

fn run_in_dir(path: &std::path::Path, dir: &std::path::Path) -> RunOutcome {
    // Imports resolve relative to the file, which lives in the chapter dir —
    // no cwd games needed; fs.* calls in snippets use relative paths rarely
    // and tolerantly (they match on Err).
    let _ = dir;
    socrates::run_capture_path(path)
}
