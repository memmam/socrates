//! The Fable programming language.
//!
//! Pipeline: [`lexer`] → [`parser`] → [`check`] (types + exhaustiveness) →
//! [`compiler`] (bytecode) → [`vm`] (execution with a mark-sweep GC).

pub mod ast;
pub mod bundle;
pub mod builtins;
pub mod bytecode;
pub mod check;
pub mod compiler;
pub mod diag;
pub mod dis;
pub mod fft;
pub mod fmt;
pub mod gpu;
pub mod jsonlite;
pub mod lsp;
/// The Objective-C runtime dispatch layer shared by every macOS raw-FFI
/// backend (see the module docs; extraction per CLAUDE.md's shared-core
/// rule).
#[cfg(all(
    any(feature = "gl", feature = "metal"),
    target_os = "macos",
    target_arch = "aarch64"
))]
pub(crate) mod objc;
pub mod modules;
/// Raw-FFI Metal primitives shared by the graphics backend and the native
/// compute path (see the module docs; extraction per CLAUDE.md's
/// shared-core rule).
#[cfg(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))]
pub(crate) mod mtl;
pub mod natives;
pub mod patterns;
pub mod repl;
pub mod lexer;
pub mod parser;
pub mod source;
pub mod stdlib;
pub mod testing;
pub mod span;
pub mod token;
pub mod types;
pub mod value;
/// Raw-FFI OpenCL compute (see the module docs; the roadmap's third native
/// compute backend and second SPIR-V consumer). Only compiled when it is
/// the *active* backend — vulkan takes precedence when both features are on.
#[cfg(all(
    feature = "opencl",
    not(feature = "vulkan"),
    any(target_os = "linux", target_os = "windows")
))]
pub(crate) mod cl;
/// Raw-FFI Vulkan compute primitives (see the module docs; the roadmap's
/// second native compute backend and first SPIR-V consumer).
#[cfg(all(feature = "vulkan", any(target_os = "linux", target_os = "windows")))]
pub(crate) mod vk;
pub mod vm;
pub mod window;
pub mod worker;

use diag::Diagnostic;

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
    // Workers spawned by the program under test write into the same buffer.
    vm.worker_sink = Some(std::sync::Arc::new(std::sync::Mutex::new(SharedWriter(
        buf.clone(),
    ))));
    let result = vm.run_entry();
    let output = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
    match result {
        Ok(_) => RunOutcome::Ok { stdout: output, warnings: diags },
        Err(e) => RunOutcome::Panic { stdout: output, error: e },
    }
}

/// Run a program from a file path, loading any imported modules relative to
/// it, capturing output. Used by the golden test harness; behavior for
/// single-file programs matches `run_capture`.
///
/// Runs the program on a spawned 512 MiB-stack thread (deep-but-legal
/// programs need it; see `src/main.rs`'s `main`) — **except** when this is
/// already the macOS main thread of a windowing-capable build, where the
/// program runs inline instead. AppKit hard-requires `NSWindow` creation on
/// the process's real main thread, so `fable test`'s windowed goldens
/// (demos/glcube's `main_metal.fable`, found failing on real macos-14
/// hardware exactly this way) only render if the test body stays on it; the
/// main thread's own 512 MiB stack comes from `build.rs`'s linker flag, so
/// nothing is lost by not spawning. The conditional matters in the other
/// direction too: under `cargo test`, these run on libtest worker threads
/// whose stacks are small — there the spawn IS the deep-stack provision and
/// must stay (windowing tests already skip gracefully off the main thread).
pub fn run_capture_path(path: &std::path::Path) -> RunOutcome {
    #[cfg(all(
        any(feature = "gl", feature = "metal"),
        target_os = "macos",
        target_arch = "aarch64"
    ))]
    if window::macos::is_main_thread() {
        return run_capture_path_here(path);
    }
    let path = path.to_path_buf();
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || run_capture_path_here(&path))
        .expect("failed to spawn interpreter thread")
        .join()
        .expect("interpreter thread panicked")
}

/// `run_capture_path` with an explicit module search path (for tests; the
/// default reads `FABLE_PATH`). Same conditional main-thread inlining as
/// `run_capture_path` — see its doc comment.
pub fn run_capture_path_with(
    path: &std::path::Path,
    search: &[std::path::PathBuf],
) -> RunOutcome {
    #[cfg(all(
        any(feature = "gl", feature = "metal"),
        target_os = "macos",
        target_arch = "aarch64"
    ))]
    if window::macos::is_main_thread() {
        return run_capture_path_with_here(path, search);
    }
    let path = path.to_path_buf();
    let search = search.to_vec();
    std::thread::Builder::new()
        .stack_size(512 * 1024 * 1024)
        .spawn(move || run_capture_path_with_here(&path, &search))
        .expect("failed to spawn interpreter thread")
        .join()
        .expect("interpreter thread panicked")
}

fn run_capture_path_with_here(path: &std::path::Path, search: &[std::path::PathBuf]) -> RunOutcome {
    let units = match modules::load_modules_with(path, search) {
        Ok(u) => u,
        Err((_source, diags)) => return RunOutcome::CompileError(diags),
    };
    run_units(units, path.parent().map(|p| p.to_path_buf()))
}

fn run_capture_path_here(path: &std::path::Path) -> RunOutcome {
    let units = match modules::load_modules(path) {
        Ok(u) => u,
        Err((_source, diags)) => return RunOutcome::CompileError(diags),
    };
    run_units(units, path.parent().map(|p| p.to_path_buf()))
}

fn run_units(units: Vec<modules::ModuleUnit>, entry_dir: Option<std::path::PathBuf>) -> RunOutcome {
    let mut warnings = Vec::new();
    let mut checker = check::Checker::new();
    let mut builder = compiler::ProgramBuilder::new();
    let mut entries = Vec::new();
    let mut final_program = None;
    for (i, unit) in units.iter().enumerate() {
        checker.check_module(&unit.program, &unit.prefix, unit.imports.clone());
        let diags = checker.take_diags();
        if diag::has_errors(&diags) {
            return RunOutcome::CompileError(diags);
        }
        warnings.extend(diags);
        let compiled = builder.compile_chunk(&unit.program, &checker, i as u32);
        entries.push(compiled.entry);
        final_program = Some(compiled);
    }
    let program = final_program.expect("loader returns at least the root module");

    let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let writer = SharedWriter(buf.clone());
    let mut units = units;
    let first_source = units.remove(0).source;
    let mut vm = vm::Vm::new(program, first_source, Box::new(writer));
    vm.entry_dir = entry_dir;
    // Workers spawned by the program under test write into the same buffer.
    vm.worker_sink = Some(std::sync::Arc::new(std::sync::Mutex::new(SharedWriter(
        buf.clone(),
    ))));
    for unit in units {
        vm.sources.push(unit.source);
    }
    let mut result = Ok(value::Value::Unit);
    for entry in entries {
        result = vm.run_entry_at(entry);
        if result.is_err() {
            break;
        }
    }
    let output = String::from_utf8_lossy(&buf.lock().unwrap()).into_owned();
    match result {
        Ok(_) => RunOutcome::Ok { stdout: output, warnings },
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
