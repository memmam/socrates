//! The Fable programming language.
//!
//! Pipeline: [`lexer`] → [`parser`] → [`check`] (types + exhaustiveness) →
//! [`compiler`] (bytecode) → [`vm`] (execution with a mark-sweep GC).

pub mod ast;
pub mod builtins;
pub mod bytecode;
pub mod check;
pub mod compiler;
pub mod diag;
pub mod dis;
pub mod fmt;
pub mod natives;
pub mod patterns;
pub mod repl;
pub mod lexer;
pub mod parser;
pub mod source;
pub mod span;
pub mod token;
pub mod types;
pub mod value;
pub mod vm;

use diag::Diagnostic;

/// Convenience pipeline: source text → compiled program + checker (or
/// diagnostics on any error). Used by the CLI, tests, and tools.
pub fn build(name: &str, text: &str) -> Result<(bytecode::CompiledProgram, check::Checker), Vec<Diagnostic>> {
    let lexed = lexer::lex(text);
    let parsed = parser::parse(lexed.tokens, text);
    let mut diags = lexed.diags;
    diags.extend(parsed.diags);
    if diag::has_errors(&diags) {
        return Err(diags);
    }
    let mut checker = check::Checker::new();
    checker.check_program(&parsed.program);
    let check_diags = checker.take_diags();
    diags.extend(check_diags);
    if diag::has_errors(&diags) {
        return Err(diags);
    }
    let program = compiler::compile(&parsed.program, &checker);
    let _ = name;
    Ok((program, checker))
}

/// Run source to completion, capturing output. Returns (stdout, Result).
/// Intended for the golden test harness. Runs on a dedicated large-stack
/// thread, mirroring the CLI (deep programs need Rust stack headroom).
pub fn run_capture(name: &str, text: &str) -> RunOutcome {
    let name = name.to_string();
    let text = text.to_string();
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || run_capture_here(&name, &text))
        .expect("failed to spawn interpreter thread")
        .join()
        .expect("interpreter thread panicked")
}

fn run_capture_here(name: &str, text: &str) -> RunOutcome {
    let lexed = lexer::lex(text);
    let parsed = parser::parse(lexed.tokens, text);
    let mut diags = lexed.diags;
    diags.extend(parsed.diags);
    if diag::has_errors(&diags) {
        return RunOutcome::CompileError(diags);
    }
    let mut checker = check::Checker::new();
    checker.check_program(&parsed.program);
    diags.extend(checker.take_diags());
    if diag::has_errors(&diags) {
        return RunOutcome::CompileError(diags);
    }
    let program = compiler::compile(&parsed.program, &checker);
    let source = source::Source::new(name, text);
    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let writer = SharedWriter(buf.clone());
    let mut vm = vm::Vm::new(program, source, Box::new(writer));
    let result = vm.run_entry();
    let output = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
    match result {
        Ok(_) => RunOutcome::Ok { stdout: output, warnings: diags },
        Err(e) => RunOutcome::Panic { stdout: output, error: e },
    }
}

pub enum RunOutcome {
    Ok { stdout: String, warnings: Vec<Diagnostic> },
    Panic { stdout: String, error: vm::VmError },
    CompileError(Vec<Diagnostic>),
}

/// A writer that appends to a shared buffer (test harness output capture).
pub struct SharedWriter(pub std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
