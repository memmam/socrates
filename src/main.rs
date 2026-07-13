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
use fable::{check, compiler, diag, dis, fmt, lexer, parser, repl, vm};

fn main() -> ExitCode {
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

    let Some(path) = rest.first() else {
        eprintln!("fable: `{cmd}` needs a file argument");
        return ExitCode::from(64);
    };
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
            let lexed = lexer::lex(&text);
            let parsed = parser::parse(lexed.tokens, &text);
            let mut diags = lexed.diags;
            diags.extend(parsed.diags);
            if diag::has_errors(&diags) {
                report(&diags, &source, color);
                return ExitCode::from(65);
            }
            let mut checker = check::Checker::new();
            checker.check_program(&parsed.program);
            diags.extend(checker.take_diags());
            report(&diags, &source, color);
            if diag::has_errors(&diags) {
                return ExitCode::from(65);
            }
            if cmd == "check" {
                let n_warn = diags.len();
                if n_warn == 0 {
                    eprintln!("ok: no errors");
                }
                return ExitCode::SUCCESS;
            }
            let program = compiler::compile(&parsed.program, &checker);
            if cmd == "dis" {
                print!("{}", dis::disassemble(&program));
                return ExitCode::SUCCESS;
            }
            let mut machine = vm::Vm::new(program, source, Box::new(std::io::stdout()));
            match machine.run_entry() {
                Ok(_) => ExitCode::SUCCESS,
                Err(e) => {
                    eprint!("{}", e.render(color));
                    ExitCode::from(70)
                }
            }
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
