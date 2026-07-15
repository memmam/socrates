//! The `fable` command-line interface.
//!
//! Usage:
//!   fable <file.fable>          compile and run (also: fable run <file>)
//!   fable check <file>          type-check only
//!   fable dis <file>            disassemble compiled bytecode
//!   fable fmt <file> [--write] [--width N]
//!                               format (print, or rewrite in place)
//!   fable tokens <file>         debug: dump tokens
//!   fable ast <file>            debug: dump the AST
//!   fable repl                  interactive session
//!
//! Exit codes: 0 success, 64 usage, 65 compile error, 70 runtime panic.

use std::io::IsTerminal;
use std::process::ExitCode;

use fable::source::Source;
use fable::{check, compiler, diag, dis, fmt, lexer, parser, modules, repl, vm};

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

    // `fmt` takes `--width N` / `--width=N`; strip it (and its value) before
    // the path scan below so the value is not mistaken for the file.
    let mut rest: Vec<String> = rest.to_vec();
    let mut fmt_width = fmt::DEFAULT_WIDTH;
    if cmd == "fmt" {
        let mut kept = Vec::with_capacity(rest.len());
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
            } else {
                kept.push(arg.clone());
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
        rest = kept;
    }
    let rest = &rest[..];

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
        "fmt" => match fmt::format_source_width(path, &text, fmt_width) {
            Ok(formatted) => {
                if rest.iter().any(|a| a == "--write" || a == "-w") {
                    if formatted != text {
                        if let Err(e) = std::fs::write(path, &formatted) {
                            eprintln!("fable: cannot write {path}: {e}");
                            return ExitCode::from(66);
                        }
                        eprintln!("formatted {path}");
                    }
                } else {
                    print!("{formatted}");
                }
                ExitCode::SUCCESS
            }
            Err(diags) => {
                report(&diags, &source, color);
                ExitCode::from(65)
            }
        },
        "check" | "dis" | "run" => {
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
            let mut entries = Vec::new();
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
                entries.push(compiled.entry);
                final_program = Some(compiled);
            }
            let program = final_program.expect("loader returns at least one module");
            if cmd == "check" {
                if !any_warnings {
                    eprintln!("ok: no errors");
                }
                return ExitCode::SUCCESS;
            }
            if cmd == "dis" {
                print!("{}", dis::disassemble(&program));
                return ExitCode::SUCCESS;
            }
            let mut units = units;
            let first_source = units.remove(0).source;
            let mut machine =
                vm::Vm::new(program, first_source, Box::new(std::io::stdout()));
            machine.script_args = script_args;
            machine.entry_dir =
                std::path::Path::new(path).parent().map(|p| p.to_path_buf());
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
        _ => unreachable!(),
    }
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
    fable fmt <file.fable> [-w] [--width N]
                                  format source (print, or -w to rewrite;
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
