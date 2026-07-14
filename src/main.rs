//! The `fable` command-line interface.
//!
//! Usage:
//!   fable <file.fable>          compile and run (also: fable run <file>)
//!   fable check <file>          type-check only
//!   fable dis <file>            disassemble compiled bytecode
//!   fable fmt <file> [--write]  format (print, or rewrite in place)
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
        Some("run") | Some("check") | Some("dis") | Some("fmt") | Some("tokens")
        | Some("ast") => (args[0].as_str(), &args[1..]),
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
        "fmt" => match fmt::format_source(path, &text) {
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
    fable fmt <file.fable> [-w]   format source (print, or -w to rewrite)
    fable tokens <file.fable>     dump tokens (debug)
    fable ast <file.fable>        dump the AST (debug)
    fable repl                    interactive session

ENVIRONMENT:
    FABLE_GC_STRESS=1    collect garbage before every allocation
    FABLE_GC_LOG=1       log collections to stderr"
    );
}
