//! The golden-test runner: any `.soc` file with expectation directives in
//! its comments is a test. Shared by the `socrates test` CLI command and the
//! interpreter's own spec suite.
//!
//! Directives (each on its own line, anywhere in the file):
//!   //? expect: <line>     — expected stdout, one directive per line, in order.
//!   //? error: <substring> — compilation must fail; some diagnostic
//!                            (message or code) must contain <substring>.
//!   //? panic: <substring> — the program must panic at runtime; the panic
//!                            message must contain <substring>.
//!
//! A directive must begin the line's comment: `//?` inside a string literal
//! (even one nested in an interpolation hole), inside a `/* */` block
//! comment, or later in the text of an ordinary `//` comment, is not a
//! directive (so prose *about* directives can't inject phantom
//! expectations). Expected and
//! actual lines are compared ignoring trailing whitespace — trailing spaces
//! in a directive are invisible in an editor and can't be pinned reliably.
//!
//! A file with no directives must merely run to completion with no output.
//! `expect` cannot be combined with `error`; `expect` + `panic` means the
//! output-so-far must match before the panic.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::RunOutcome;

#[derive(Default)]
pub struct Directives {
    pub expect: Vec<String>,
    pub errors: Vec<String>,
    pub panics: Vec<String>,
}

/// Per-line lexical mode for the directive scanner. Strings (and their
/// interpolation holes) cannot span lines in Socrates, so only the block-comment
/// depth carries across lines.
enum ScanMode {
    /// Inside a string literal.
    Str,
    /// Inside a `{ .. }` interpolation hole; the payload is the brace depth
    /// (holes contain arbitrary expressions, including `{..}` literals).
    Hole(u32),
}

/// Where the line's directive comment starts, if any.
///
/// Tracks just enough of Socrates's lexical structure to be truthful: string
/// literals (with `\` escapes), strings nested inside `{ .. }` interpolation
/// holes (to arbitrary depth), and nested `/* */` block comments — whose
/// depth persists across lines via `block_depth`. A directive is a **line**
/// comment that starts with `//?`, found outside all of the above; the first
/// non-directive line comment ends the scan (the rest of the line is prose).
fn directive_start(line: &str, block_depth: &mut u32) -> Option<usize> {
    let b = line.as_bytes();
    let mut i = 0;
    let mut stack: Vec<ScanMode> = Vec::new();
    while i < b.len() {
        if *block_depth > 0 {
            // Inside a block comment nothing else is lexical structure.
            match b[i] {
                b'/' if b.get(i + 1) == Some(&b'*') => {
                    *block_depth += 1;
                    i += 2;
                    continue;
                }
                b'*' if b.get(i + 1) == Some(&b'/') => {
                    *block_depth -= 1;
                    i += 2;
                    continue;
                }
                _ => {
                    i += 1;
                    continue;
                }
            }
        }
        match stack.last_mut() {
            Some(ScanMode::Str) => match b[i] {
                b'\\' => {
                    i += 2;
                    continue;
                }
                b'{' => stack.push(ScanMode::Hole(1)),
                b'"' => {
                    stack.pop();
                }
                _ => {}
            },
            Some(ScanMode::Hole(depth)) => match b[i] {
                b'"' => stack.push(ScanMode::Str),
                b'{' => *depth += 1,
                b'}' => {
                    *depth -= 1;
                    if *depth == 0 {
                        stack.pop();
                    }
                }
                _ => {}
            },
            None => match b[i] {
                b'"' => stack.push(ScanMode::Str),
                b'/' if b.get(i + 1) == Some(&b'*') => {
                    *block_depth = 1;
                    i += 2;
                    continue;
                }
                b'/' if b.get(i + 1) == Some(&b'/') => {
                    return if b.get(i + 2) == Some(&b'?') { Some(i) } else { None };
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

pub fn parse_directives(text: &str) -> Directives {
    let mut d = Directives::default();
    let mut block_depth = 0u32;
    for line in text.lines() {
        let Some(idx) = directive_start(line, &mut block_depth) else { continue };
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

/// Recursively collect `.soc` files under `dir` (sorted for determinism).
pub fn collect_socrates_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            collect_socrates_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "soc") {
            out.push(p);
        }
    }
}

/// Run one test file and check it against its directives.
pub fn check_one(path: &Path) -> Result<(), String> {
    check_or_bless(path, false).map(|_| ())
}

/// What `check_or_bless` did.
pub enum CheckOutcome {
    /// Everything already matched.
    Passed,
    /// Bless mode rewrote `expect:` lines to match actual output.
    Blessed(usize),
}

/// Run one test file and check it against its directives. When `bless` is
/// true, a stdout-only mismatch whose actual/expected line counts already
/// agree is silently fixed by rewriting the file's `//? expect:` lines
/// in place instead of failing — the automatable half of "generate long pin
/// blocks mechanically" (demos/STYLE.md § 1). A line-count change means a
/// print statement was added or removed, which directive lines correspond
/// to which output is then ambiguous, so that case still fails normally
/// (compile-error and panic-message mismatches are never blessed either —
/// `expect:` is the only directive this rewrites).
pub fn check_or_bless(path: &Path, bless: bool) -> Result<CheckOutcome, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read failed: {e}"))?;
    let d = parse_directives(&text);
    // Path-based so `import` resolves sibling files and SOCRATES_PATH.
    let outcome = crate::run_capture_path(path);

    let stdout = match outcome {
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
            return Ok(CheckOutcome::Passed);
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
            stdout
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
            stdout
        }
    };

    match check_stdout(&d.expect, &stdout) {
        Ok(()) => Ok(CheckOutcome::Passed),
        Err(msg) => {
            if bless {
                if let Some(n) = bless_expect(path, &text, &d.expect, &stdout)? {
                    return Ok(CheckOutcome::Blessed(n));
                }
            }
            Err(msg)
        }
    }
}

/// Rewrite every `//? expect:` line's payload to the corresponding actual
/// output line, in file order. `None` (not an error) when the line counts
/// differ — nothing is written, and the caller keeps the original mismatch
/// message, which already shows both lengths.
fn bless_expect(
    path: &Path,
    text: &str,
    expect: &[String],
    stdout: &str,
) -> Result<Option<usize>, String> {
    let got: Vec<&str> = stdout.lines().collect();
    if got.len() != expect.len() {
        return Ok(None);
    }
    let mut block_depth = 0u32;
    let mut k = 0usize;
    let mut changed = 0usize;
    let mut out_lines: Vec<String> = Vec::with_capacity(text.lines().count());
    for line in text.lines() {
        if let Some(idx) = directive_start(line, &mut block_depth) {
            if line[idx + 3..].trim_start().starts_with("expect:") {
                let new_val = got[k].trim_end();
                let new_line = format!("{}//? expect: {}", &line[..idx], new_val);
                if new_line != line {
                    changed += 1;
                }
                out_lines.push(new_line);
                k += 1;
                continue;
            }
        }
        out_lines.push(line.to_string());
    }
    let mut new_text = out_lines.join("\n");
    if text.ends_with('\n') {
        new_text.push('\n');
    }
    std::fs::write(path, new_text).map_err(|e| format!("bless: write failed: {e}"))?;
    Ok(Some(changed))
}

fn check_stdout(expect: &[String], stdout: &str) -> Result<(), String> {
    let got: Vec<&str> = stdout.lines().collect();
    if got.len() != expect.len()
        || !got
            .iter()
            .zip(expect)
            .all(|(g, e)| g.trim_end() == e.trim_end())
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

pub struct TestReport {
    pub total: usize,
    pub failures: Vec<(PathBuf, String)>,
    /// Files whose `//? expect:` lines were rewritten (`--bless`), with the
    /// count of lines changed in each. Always empty unless bless mode ran.
    pub blessed: Vec<(PathBuf, usize)>,
}

/// Run every `.soc` file under the given paths (files are taken as-is,
/// directories are walked) and report failures.
pub fn run_test_paths(paths: &[PathBuf]) -> TestReport {
    run_test_paths_bless(paths, false)
}

/// `run_test_paths`, optionally in `--bless` mode (§ `check_or_bless`).
pub fn run_test_paths_bless(paths: &[PathBuf], bless: bool) -> TestReport {
    let mut files = Vec::new();
    for p in paths {
        if p.is_dir() {
            collect_socrates_files(p, &mut files);
        } else {
            files.push(p.clone());
        }
    }
    let mut failures = Vec::new();
    let mut blessed = Vec::new();
    for f in &files {
        match check_or_bless(f, bless) {
            Ok(CheckOutcome::Passed) => {}
            Ok(CheckOutcome::Blessed(n)) => blessed.push((f.clone(), n)),
            Err(msg) => failures.push((f.clone(), msg)),
        }
    }
    TestReport { total: files.len(), failures, blessed }
}
