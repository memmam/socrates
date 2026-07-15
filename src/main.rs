//! The `fable` command-line interface.
//!
//! Usage:
//!   fable <file.fable>          compile and run (also: fable run <file>)
//!   fable check <file>          type-check only
//!   fable dis <file>            disassemble compiled bytecode
//!   fable build <dir|file> [-o OUT] [--launcher PATH]
//!                               staple a program into a self-contained binary
//!   fable fmt <file>... [--write] [--width N]
//!                               format each file (print, or rewrite in place)
//!   fable tokens <file>         debug: dump tokens
//!   fable ast <file>            debug: dump the AST
//!   fable repl                  interactive session
//!
//! A binary produced by `fable build` carries its program's files stapled to
//! this executable; on startup [`fable::bundle::read_self`] finds them and we
//! run the entry instead of dispatching a subcommand (see `run_bundle`).
//!
//! Exit codes: 0 success, 64 usage, 65 compile error, 70 runtime panic.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fable::source::Source;
use fable::{bundle, check, compiler, diag, dis, fmt, lexer, parser, modules, repl, vm};

fn main() -> ExitCode {
    // Deep-but-legal programs (nested expressions, native callback
    // re-entrancy, deep value display) consume Rust stack proportional to
    // their depth; a large virtual stack keeps Fable's own limits (4096
    // frames, parser/checker nesting caps) the binding ones.
    std::thread::Builder::new()
        .name("fable".into())
        .stack_size(512 * 1024 * 1024)
        .spawn(real_main)
        .expect("failed to spawn interpreter thread")
        .join()
        .expect("interpreter thread panicked")
}

fn real_main() -> ExitCode {
    // A stapled binary runs its embedded program and never dispatches
    // subcommands — its argv belong to the program. An ordinary `fable` has
    // no trailer, so this is one 16-byte read and we fall through to the CLI.
    if let Some(b) = bundle::read_self() {
        return run_bundle(b);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, rest) = match args.first().map(|s| s.as_str()) {
        None => {
            if std::io::stdin().is_terminal() {
                return ExitCode::from(repl::run_repl() as u8);
            }
            usage();
            return ExitCode::from(64);
        }
        Some("repl") => return ExitCode::from(repl::run_repl() as u8),
        Some("lsp") => return ExitCode::from(fable::lsp::run_lsp() as u8),
        Some("build") => return build_bundle(&args[1..]),
        Some("run") | Some("check") | Some("dis") | Some("fmt") | Some("tokens")
        | Some("ast") => (args[0].as_str(), &args[1..]),
        Some("test") => {
            // `fable test` takes no flags; silently swallowing `--help` (and
            // then walking the whole cwd) helps no one. A literal `--` ends
            // flag checking so dash-prefixed paths can still be named.
            let rest = &args[1..];
            let sep = rest.iter().position(|a| a == "--");
            let flag_zone = &rest[..sep.unwrap_or(rest.len())];
            if let Some(flag) = flag_zone.iter().find(|a| a.starts_with('-')) {
                eprintln!("fable test: unknown flag `{flag}`");
                eprintln!(
                    "usage: fable test [paths...]   (files or directories; default `.`; `--` ends flags)"
                );
                return ExitCode::from(64);
            }
            let paths: Vec<std::path::PathBuf> = rest
                .iter()
                .enumerate()
                .filter(|(i, _)| sep != Some(*i))
                .map(|(_, a)| std::path::PathBuf::from(a))
                .collect();
            let paths = if paths.is_empty() {
                vec![std::path::PathBuf::from(".")]
            } else {
                paths
            };
            let color = std::io::stderr().is_terminal();
            let (green, red, bold, reset) = if color {
                ("\x1b[32m", "\x1b[1;31m", "\x1b[1m", "\x1b[0m")
            } else {
                ("", "", "", "")
            };
            let report = fable::testing::run_test_paths(&paths);
            if report.total == 0 {
                eprintln!("fable test: no .fable files found");
                return ExitCode::from(64);
            }
            for (path, why) in &report.failures {
                eprintln!("{red}FAIL{reset} {bold}{}{reset}", path.display());
                for line in why.lines() {
                    eprintln!("     {line}");
                }
            }
            let passed = report.total - report.failures.len();
            if report.failures.is_empty() {
                eprintln!("{green}ok{reset}: {passed} test{} passed", if passed == 1 { "" } else { "s" });
                return ExitCode::SUCCESS;
            }
            eprintln!(
                "{red}FAILED{reset}: {} of {} tests failed",
                report.failures.len(),
                report.total
            );
            return ExitCode::from(1);
        }
        Some("--help") | Some("-h") | Some("help") => {
            usage();
            return ExitCode::SUCCESS;
        }
        Some("--version") | Some("-V") => {
            println!("fable {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Some(path) if path.ends_with(".fable") || std::path::Path::new(path).exists() => {
            ("run", &args[0..])
        }
        Some(other) => {
            eprintln!("fable: unknown command `{other}`\n");
            usage();
            return ExitCode::from(64);
        }
    };

    // `fable fmt [-w] [--width N] <file.fable>...`: flags may appear anywhere
    // among the files, and every named file is formatted. A file that fails
    // to read or parse is reported and the rest still format; the exit code
    // is then nonzero (65 for parse errors, 66 for I/O errors).
    if cmd == "fmt" {
        let mut fmt_width = fmt::DEFAULT_WIDTH;
        let mut write = false;
        let mut files: Vec<String> = Vec::new();
        let mut i = 0;
        while i < rest.len() {
            let arg = &rest[i];
            let value = if arg == "--width" {
                i += 1;
                match rest.get(i) {
                    Some(v) => Some(v.as_str()),
                    None => {
                        eprintln!("fable fmt: `--width` needs a number");
                        return ExitCode::from(64);
                    }
                }
            } else if let Some(v) = arg.strip_prefix("--width=") {
                Some(v)
            } else if arg == "--write" || arg == "-w" {
                write = true;
                None
            } else if arg.starts_with('-') {
                eprintln!("fable fmt: unknown flag `{arg}`");
                eprintln!("usage: fable fmt [-w] [--width N] <file.fable>...");
                return ExitCode::from(64);
            } else {
                files.push(arg.clone());
                None
            };
            if let Some(v) = value {
                match v.parse::<usize>() {
                    Ok(n) if n > 0 => fmt_width = n,
                    _ => {
                        eprintln!("fable fmt: invalid width `{v}` (need a positive integer)");
                        return ExitCode::from(64);
                    }
                }
            }
            i += 1;
        }
        if files.is_empty() {
            eprintln!("fable fmt: needs at least one file argument");
            eprintln!("usage: fable fmt [-w] [--width N] <file.fable>...");
            return ExitCode::from(64);
        }
        let color = std::io::stderr().is_terminal();
        let mut io_error = false;
        let mut parse_error = false;
        for path in &files {
            let text = match std::fs::read_to_string(path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("fable: cannot read {path}: {e}");
                    io_error = true;
                    continue;
                }
            };
            match fmt::format_source_width(path, &text, fmt_width) {
                Ok(formatted) => {
                    if write {
                        if formatted != text {
                            if let Err(e) = std::fs::write(path, &formatted) {
                                eprintln!("fable: cannot write {path}: {e}");
                                io_error = true;
                                continue;
                            }
                            eprintln!("formatted {path}");
                        }
                    } else {
                        print!("{formatted}");
                    }
                }
                Err(diags) => {
                    let source = Source::new(path.clone(), text);
                    report(&diags, &source, color);
                    parse_error = true;
                }
            }
        }
        return if parse_error {
            ExitCode::from(65)
        } else if io_error {
            ExitCode::from(66)
        } else {
            ExitCode::SUCCESS
        };
    }

    let Some(path_pos) = rest.iter().position(|a| !a.starts_with('-')) else {
        eprintln!("fable: `{cmd}` needs a file argument");
        return ExitCode::from(64);
    };
    let path = &rest[path_pos];
    // Everything after the script path belongs to the script (`os.args()`).
    let script_args: Vec<String> = rest[path_pos + 1..].to_vec();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("fable: cannot read {path}: {e}");
            return ExitCode::from(66);
        }
    };
    let color = std::io::stderr().is_terminal();
    let source = Source::new(path.clone(), text.clone());

    match cmd {
        "tokens" => {
            let lexed = lexer::lex(&text);
            for t in &lexed.tokens {
                let lc = source.line_col(t.span.start);
                println!("{}:{}\t{:?}", lc.line, lc.col, t.kind);
            }
            report(&lexed.diags, &source, color);
            exit_for(&lexed.diags)
        }
        "ast" => {
            let lexed = lexer::lex(&text);
            let parsed = parser::parse(lexed.tokens, &text);
            let mut diags = lexed.diags;
            diags.extend(parsed.diags);
            println!("{:#?}", parsed.program);
            report(&diags, &source, color);
            exit_for(&diags)
        }
        "run" => run_path(path, script_args, color),
        "check" | "dis" => {
            // Load the root file and everything it imports (a single-file
            // program is just a one-module load).
            let units = match modules::load_modules(std::path::Path::new(path)) {
                Ok(u) => u,
                Err((src, diags)) => {
                    report(&diags, &src, color);
                    return ExitCode::from(65);
                }
            };
            let mut checker = check::Checker::new();
            let mut builder = compiler::ProgramBuilder::new();
            let mut final_program = None;
            let mut any_warnings = false;
            for (i, unit) in units.iter().enumerate() {
                checker.check_module(&unit.program, &unit.prefix, unit.imports.clone());
                let diags = checker.take_diags();
                report(&diags, &unit.source, color);
                if diag::has_errors(&diags) {
                    return ExitCode::from(65);
                }
                any_warnings |= !diags.is_empty();
                let compiled = builder.compile_chunk(&unit.program, &checker, i as u32);
                final_program = Some(compiled);
            }
            let program = final_program.expect("loader returns at least one module");
            if cmd == "check" {
                if !any_warnings {
                    eprintln!("ok: no errors");
                }
                return ExitCode::SUCCESS;
            }
            // dis
            print!("{}", dis::disassemble(&program));
            ExitCode::SUCCESS
        }
        _ => unreachable!(),
    }
}

/// Load, type-check, compile, and run the program rooted at `path`,
/// streaming to real stdout. Backs both the `run` subcommand and the bundle
/// launcher. Exit codes: 65 compile error, 70 runtime panic, 0 success.
fn run_path(path: &str, script_args: Vec<String>, color: bool) -> ExitCode {
    let units = match modules::load_modules(Path::new(path)) {
        Ok(u) => u,
        Err((src, diags)) => {
            report(&diags, &src, color);
            return ExitCode::from(65);
        }
    };
    let mut checker = check::Checker::new();
    let mut builder = compiler::ProgramBuilder::new();
    let mut entries = Vec::new();
    let mut final_program = None;
    for (i, unit) in units.iter().enumerate() {
        checker.check_module(&unit.program, &unit.prefix, unit.imports.clone());
        let diags = checker.take_diags();
        report(&diags, &unit.source, color);
        if diag::has_errors(&diags) {
            return ExitCode::from(65);
        }
        let compiled = builder.compile_chunk(&unit.program, &checker, i as u32);
        entries.push(compiled.entry);
        final_program = Some(compiled);
    }
    let program = final_program.expect("loader returns at least one module");
    let mut units = units;
    let first_source = units.remove(0).source;
    let mut machine = vm::Vm::new(program, first_source, Box::new(std::io::stdout()));
    machine.script_args = script_args;
    machine.entry_dir = Path::new(path).parent().map(|p| p.to_path_buf());
    for unit in units {
        machine.sources.push(unit.source);
    }
    for entry in entries {
        if let Err(e) = machine.run_entry_at(entry) {
            eprint!("{}", e.render(color));
            return ExitCode::from(70);
        }
    }
    ExitCode::SUCCESS
}

/// Run the program stapled onto this binary. The files are unpacked into a
/// per-process scratch directory that becomes the working directory, so the
/// program's relative paths (imports, `fs.*`, worker files, outputs) resolve
/// against the unpacked tree exactly as they would from a source checkout.
fn run_bundle(b: bundle::Bundle) -> ExitCode {
    let color = std::io::stderr().is_terminal();
    let dir = std::env::temp_dir().join(format!("fable-zoo-{}", std::process::id()));
    // Clear any stale remnant from a prior run that reused this pid.
    let _ = std::fs::remove_dir_all(&dir);
    let entry = match std::fs::create_dir_all(&dir).and_then(|()| b.extract_to(&dir)) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("fable: cannot unpack bundled program: {e}");
            let _ = std::fs::remove_dir_all(&dir);
            return ExitCode::from(70);
        }
    };
    if let Err(e) = std::env::set_current_dir(&dir) {
        eprintln!("fable: cannot enter scratch directory {}: {e}", dir.display());
        let _ = std::fs::remove_dir_all(&dir);
        return ExitCode::from(70);
    }
    let script_args: Vec<String> = std::env::args().skip(1).collect();
    let code = run_path(&entry.to_string_lossy(), script_args, color);
    // Step out before removing the tree (Windows locks the working dir).
    let _ = std::env::set_current_dir(std::env::temp_dir());
    let _ = std::fs::remove_dir_all(&dir);
    code
}

/// `fable build <dir|file> [-o OUT] [--launcher PATH]` — staple a program's
/// files onto a copy of the interpreter, producing a self-contained binary.
/// The program is type-checked first, so a broken program fails here rather
/// than shipping a binary that panics on launch.
fn build_bundle(rest: &[String]) -> ExitCode {
    let mut input: Option<String> = None;
    let mut out: Option<String> = None;
    let mut launcher: Option<String> = None;
    let mut payload_only = false;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                match rest.get(i) {
                    Some(v) => out = Some(v.clone()),
                    None => return usage_err("`-o` needs an output path"),
                }
            }
            "--launcher" => {
                i += 1;
                match rest.get(i) {
                    Some(v) => launcher = Some(v.clone()),
                    None => return usage_err("`--launcher` needs a path"),
                }
            }
            // Emit just the raw payload archive instead of a stapled binary.
            // The macOS release links this into a binary as a Mach-O section
            // (`ld -sectcreate`), which appending can't achieve there.
            "--payload-only" => payload_only = true,
            flag if flag.starts_with('-') && flag != "-" => {
                return usage_err(&format!("unknown flag `{flag}`"));
            }
            positional => {
                if input.is_some() {
                    return usage_err("build takes a single program (a directory or a .fable file)");
                }
                input = Some(positional.to_string());
            }
        }
        i += 1;
    }
    let Some(input) = input else {
        return usage_err("build needs a program (a directory or a .fable file)");
    };

    // Resolve: `root` = the directory to pack, `prefix` = the bundle path
    // that directory sits under, `entry_name` = the entry file within it.
    // The prefix mirrors the path *as given* (e.g. `demos/png`), so at
    // runtime the program sits at the same relative place it had when built —
    // the stapled binary then behaves exactly like `fable <that path>` run
    // from the build directory, with no per-program path assumptions to
    // reproduce. See `run_bundle`.
    let inp = Path::new(&input);
    let (root, prefix, entry_name) = if inp.is_dir() {
        (inp.to_path_buf(), clean_prefix(&input), "main.fable".to_string())
    } else if inp.is_file() {
        let dir = inp.parent().filter(|p| !p.as_os_str().is_empty()).map_or_else(
            || PathBuf::from("."),
            Path::to_path_buf,
        );
        let dir_str = inp.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let file = inp.file_name().unwrap().to_string_lossy().into_owned();
        (dir, clean_prefix(&dir_str), file)
    } else {
        eprintln!("fable build: no such file or directory: {input}");
        return ExitCode::from(66);
    };
    let entry_path = root.join(&entry_name);
    if !entry_path.is_file() {
        eprintln!("fable build: no entry `{}` in {}", entry_name, root.display());
        eprintln!("  (point at a directory containing main.fable, or at a .fable file)");
        return ExitCode::from(64);
    }
    let entry_rel = join_prefix(&prefix, &entry_name);

    // Type-check the program before packing it — building a binary that only
    // fails once launched helps no one.
    let color = std::io::stderr().is_terminal();
    if let Err(code) = check_only(&entry_path, color) {
        eprintln!("fable build: not packing a program that does not compile");
        return code;
    }

    // Collect every file under the root (subpaths relative to it), then move
    // them under the bundle prefix, deterministically ordered.
    let mut sub = Vec::new();
    if let Err(e) = collect_files(&root, &root, &mut sub) {
        eprintln!("fable build: cannot read {}: {e}", root.display());
        return ExitCode::from(66);
    }
    let mut files: Vec<(String, Vec<u8>)> =
        sub.into_iter().map(|(rel, data)| (join_prefix(&prefix, &rel), data)).collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let payload = bundle::payload(&entry_rel, &files);

    // `--payload-only`: emit the raw archive, no launcher. The macOS release
    // links it in as a Mach-O section instead of appending.
    if payload_only {
        let out = out.unwrap_or_else(|| "payload.bin".to_string());
        if let Err(e) = std::fs::write(&out, &payload) {
            eprintln!("fable build: cannot write {out}: {e}");
            return ExitCode::from(66);
        }
        eprintln!(
            "wrote {out} ({} file{}, {} bytes)",
            files.len(),
            if files.len() == 1 { "" } else { "s" },
            payload.len()
        );
        return ExitCode::SUCCESS;
    }

    // Launcher bytes: an explicit path (a cross-compiled `fable` for another
    // target) or this very executable.
    let launcher_bytes = match &launcher {
        Some(p) => match std::fs::read(p) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("fable build: cannot read launcher {p}: {e}");
                return ExitCode::from(66);
            }
        },
        None => {
            if bundle::read_self().is_some() {
                eprintln!("fable build: this `fable` is itself a stapled binary; pass --launcher with a plain `fable`");
                return ExitCode::from(64);
            }
            match std::env::current_exe().and_then(std::fs::read) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("fable build: cannot read this executable: {e}");
                    return ExitCode::from(66);
                }
            }
        }
    };

    let image = bundle::staple(&launcher_bytes, &payload);

    // Default output name: the program directory's basename (adopting the
    // launcher's extension, so a Windows launcher yields `<name>.exe`).
    let out = out.unwrap_or_else(|| {
        let stem = root
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .filter(|s| !s.is_empty() && s != ".")
            .unwrap_or_else(|| "program".to_string());
        match launcher.as_deref().and_then(|l| Path::new(l).extension()) {
            Some(ext) => format!("{stem}.{}", ext.to_string_lossy()),
            None => stem,
        }
    });
    if let Err(e) = std::fs::write(&out, &image) {
        eprintln!("fable build: cannot write {out}: {e}");
        return ExitCode::from(66);
    }
    make_executable(&out);
    eprintln!(
        "built {out} ({} file{}, {} KiB)",
        files.len(),
        if files.len() == 1 { "" } else { "s" },
        image.len().div_ceil(1024)
    );
    ExitCode::SUCCESS
}

/// Type-check the program at `entry` (loading its imports); `Err(code)` on a
/// load or type error. Used by `fable build` as a pre-flight.
fn check_only(entry: &Path, color: bool) -> Result<(), ExitCode> {
    let units = match modules::load_modules(entry) {
        Ok(u) => u,
        Err((src, diags)) => {
            report(&diags, &src, color);
            return Err(ExitCode::from(65));
        }
    };
    let mut checker = check::Checker::new();
    for unit in &units {
        checker.check_module(&unit.program, &unit.prefix, unit.imports.clone());
        let diags = checker.take_diags();
        report(&diags, &unit.source, color);
        if diag::has_errors(&diags) {
            return Err(ExitCode::from(65));
        }
    }
    Ok(())
}

/// Recursively collect regular files under `dir` as `(relative-path, bytes)`,
/// using `/` separators. Skips symlinks and `.git`.
fn collect_files(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, Vec<u8>)>,
) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let meta = entry.metadata()?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if entry.file_name() == ".git" {
                continue;
            }
            collect_files(root, &path, out)?;
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((rel, std::fs::read(&path)?));
        }
    }
    Ok(())
}

/// Normalize a user-given path into a safe, `/`-separated bundle prefix:
/// drop `.`/empty segments and anything that would climb or absolutize
/// (`..`, a leading `/`). `demos/png` stays `demos/png`; `.` becomes empty.
fn clean_prefix(s: &str) -> String {
    s.replace('\\', "/")
        .split('/')
        .filter(|seg| !matches!(*seg, "" | "." | ".."))
        .collect::<Vec<_>>()
        .join("/")
}

/// Join a bundle prefix onto a relative path (either may be empty).
fn join_prefix(prefix: &str, rel: &str) -> String {
    if prefix.is_empty() {
        rel.to_string()
    } else if rel.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}/{rel}")
    }
}

#[cfg(unix)]
fn make_executable(path: &str) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o111);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn make_executable(_path: &str) {}

fn usage_err(msg: &str) -> ExitCode {
    eprintln!("fable build: {msg}");
    eprintln!("usage: fable build <dir|file.fable> [-o OUT] [--launcher PATH] [--payload-only]");
    ExitCode::from(64)
}

fn report(diags: &[diag::Diagnostic], source: &Source, color: bool) {
    if !diags.is_empty() {
        eprint!("{}", diag::render(diags, source, color));
    }
}

fn exit_for(diags: &[diag::Diagnostic]) -> ExitCode {
    if diag::has_errors(diags) {
        ExitCode::from(65)
    } else {
        ExitCode::SUCCESS
    }
}

fn usage() {
    eprintln!(
        "The Fable programming language

USAGE:
    fable <file.fable>            compile and run
    fable run <file.fable>        compile and run
    fable check <file.fable>      type-check only
    fable dis <file.fable>        show compiled bytecode
    fable build <dir|file.fable> [-o OUT] [--launcher PATH]
                                  staple a program into a self-contained
                                  binary (entry: main.fable in a directory)
    fable fmt <file.fable>... [-w] [--width N]
                                  format each file (print, or -w to rewrite;
                                  N: max line width, default 100)
    fable test [paths...]         run golden tests (//? expect/error/panic
                                  directives in .fable files; default: .)
    fable tokens <file.fable>     dump tokens (debug)
    fable ast <file.fable>        dump the AST (debug)
    fable repl                    interactive session
    fable lsp                     language server (JSON-RPC over stdio)

ENVIRONMENT:
    FABLE_GC_STRESS=1    collect garbage before every allocation
    FABLE_GC_LOG=1       log collections to stderr"
    );
}
