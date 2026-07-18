//! The interactive REPL (`socrates repl`).
//!
//! Each submitted chunk is checked and compiled *incrementally*: the checker,
//! program builder, and VM persist, so definitions accumulate and closures
//! created in earlier chunks stay valid. A chunk whose check fails is rolled
//! back (checker/builder are cloned before each attempt), leaving the session
//! unpolluted. The final expression statement of a chunk is bound to a hidden
//! global and its value is printed (unless it is `()`).

use std::io::{IsTerminal, Write};

use crate::ast::{Pattern, PatternKind, Stmt, StmtKind};
use crate::check::Checker;
use crate::compiler::ProgramBuilder;
use crate::modules::ModuleSession;
use crate::diag;
use crate::source::Source;
use crate::span::NodeId;
use crate::token::TokenKind;
use crate::types::{display_type, Type};
use crate::value::{Obj, Value};
use crate::vm::Vm;

pub fn run_repl() -> i32 {
    let color = std::io::stdout().is_terminal();
    println!("Socrates {} — type a program, or :help", env!("CARGO_PKG_VERSION"));

    let mut checker = Checker::new();
    let mut builder = ProgramBuilder::new();
    let mut vm: Option<Vm> = None;
    let mut node_offset: u32 = 0;
    let mut chunk_no: u32 = 0;
    // Imports work in the REPL (v0.5): loaded modules and the session's
    // alias map persist across chunks.
    let mut modsession = ModuleSession::new();
    let mut session_imports: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let base_dir = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    let stdin = std::io::stdin();
    let mut pending = String::new();
    loop {
        let prompt = if pending.is_empty() { "socrates> " } else { "  ...> " };
        print!("{prompt}");
        let _ = std::io::stdout().flush();

        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => {
                println!();
                return 0;
            }
            Ok(_) => {}
            Err(_) => return 0,
        }
        let trimmed = line.trim();
        if pending.is_empty() {
            match trimmed {
                ":q" | ":quit" | ":exit" => return 0,
                ":help" => {
                    println!(
                        "Enter Socrates code (multi-line input continues while delimiters are open).\n\
                         Imports work: `import std.json;` or files relative to the working directory.\n\
                         Commands: :help  :type <expr>  :q"
                    );
                    continue;
                }
                "" => continue,
                _ => {}
            }
        }

        let type_query = pending.is_empty() && trimmed.starts_with(":type ");
        let code_text = if type_query {
            trimmed.trim_start_matches(":type ").to_string()
        } else {
            pending.push_str(&line);
            // Keep reading while brackets are unbalanced.
            if delimiters_open(&pending) {
                continue;
            }
            std::mem::take(&mut pending)
        };

        chunk_no += 1;
        let name = format!("<repl-{chunk_no}>");
        let lexed = crate::lexer::lex(&code_text);
        let mut diags = lexed.diags;
        let parsed = crate::parser::parse_with_ids(lexed.tokens, &code_text, node_offset);
        diags.extend(parsed.diags);
        let source = Source::new(name.clone(), code_text.clone());
        if diag::has_errors(&diags) {
            print!("{}", diag::render(&diags, &source, color));
            continue;
        }

        let mut program = parsed.program;
        let mut next_id = parsed.node_count;

        // Bind a trailing expression statement to a hidden global for display.
        let mut result_name: Option<String> = None;
        if !type_query {
            if let Some(Stmt { kind: StmtKind::Expr { expr, .. }, span, .. }) =
                program.stmts.last()
            {
                let gname = format!("__repl_{chunk_no}");
                let span = *span;
                let expr = expr.clone();
                let pat = Pattern {
                    kind: PatternKind::Binding(gname.clone()),
                    span,
                    id: NodeId(next_id),
                };
                next_id += 1;
                let stmt_id = NodeId(next_id);
                next_id += 1;
                *program.stmts.last_mut().unwrap() = Stmt {
                    kind: StmtKind::Let { is_pub: false, mutable: false, pattern: pat, ty: None, init: expr },
                    span,
                    id: stmt_id,
                };
                result_name = Some(gname);
            }
        } else {
            // For :type we only need the check, not execution.
            if let Some(Stmt { kind: StmtKind::Expr { expr, .. }, .. }) = program.stmts.last() {
                let expr_id = expr.id;
                let mut probe = checker.clone();
                probe.check_module(&program, "", session_imports.clone());
                let d = probe.take_diags();
                if diag::has_errors(&d) {
                    print!("{}", diag::render(&d, &source, color));
                } else {
                    let ty = probe.types.get(&expr_id).cloned().unwrap_or(Type::Unit);
                    println!(": {}", display_type(&ty, &probe.defs, &[]));
                }
            } else {
                println!("(not an expression)");
            }
            continue;
        }

        // Resolve any imports in the chunk (rolling the session back if
        // loading fails), then check the new units and the chunk itself.
        let saved_checker = checker.clone();
        let saved_session = modsession.clone();
        let overlay = std::collections::HashMap::new();
        let (new_units, chunk_imports, after_id) =
            match modsession.load_imports(&program, &base_dir, &source, next_id, &overlay) {
                Ok(r) => r,
                Err((err_source, diags)) => {
                    print!("{}", diag::render(&diags, &err_source, color));
                    modsession = saved_session;
                    continue;
                }
            };
        let next_id = after_id;

        let mut unit_check_failed = false;
        for unit in &new_units {
            checker.check_module(&unit.program, &unit.prefix, unit.imports.clone());
            let unit_diags = checker.take_diags();
            if diag::has_errors(&unit_diags) {
                print!("{}", diag::render(&unit_diags, &unit.source, color));
                unit_check_failed = true;
                break;
            }
        }
        if unit_check_failed {
            checker = saved_checker;
            modsession = saved_session;
            continue;
        }

        let mut merged_imports = session_imports.clone();
        merged_imports.extend(chunk_imports.clone());
        checker.check_module(&program, "", merged_imports.clone());
        let check_diags = checker.take_diags();
        print!("{}", diag::render(&check_diags, &source, color));
        if diag::has_errors(&check_diags) {
            checker = saved_checker;
            modsession = saved_session;
            continue;
        }
        node_offset = next_id;
        session_imports = merged_imports;

        // Compile and run the new modules' top-level code, then the chunk.
        for unit in new_units {
            let source_idx = vm.as_ref().map(|v| v.sources.len() as u32).unwrap_or(0);
            let compiled = builder.compile_chunk(&unit.program, &checker, source_idx);
            match &mut vm {
                None => {
                    vm = Some(Vm::new(compiled, unit.source, Box::new(std::io::stdout())));
                }
                Some(vm) => vm.update_program(compiled, unit.source),
            }
            if let Err(e) = vm.as_mut().unwrap().run_entry() {
                print!("{}", e.render(color));
            }
        }

        let source_idx = vm.as_ref().map(|v| v.sources.len() as u32).unwrap_or(0);
        let compiled = builder.compile_chunk(&program, &checker, source_idx);
        match &mut vm {
            None => {
                vm = Some(Vm::new(compiled, source, Box::new(std::io::stdout())));
            }
            Some(vm) => vm.update_program(compiled, source),
        }
        let vm = vm.as_mut().unwrap();
        match vm.run_entry() {
            Ok(_) => {
                if let Some(gname) = result_name {
                    let slot = checker
                        .globals
                        .iter()
                        .rposition(|g| g.name == gname);
                    if let Some(slot) = slot {
                        let v = vm.globals[slot];
                        let ty = &checker.globals[slot].ty;
                        if !matches!(ty, Type::Unit) && !matches!(v, Value::Undefined) {
                            match repl_display(vm, v) {
                                Ok(shown) => println!(
                                    "{} : {}",
                                    shown,
                                    display_type(ty, &checker.defs, &[])
                                ),
                                Err(e) => print!("{}", e.render(color)),
                            }
                        }
                    }
                }
            }
            Err(e) => {
                print!("{}", e.render(color));
                // Note: runtime state may be partially updated; the session
                // continues (like most REPLs).
            }
        }
    }
}

/// REPL display: like `str(x)` but top-level strings are quoted.
fn repl_display(vm: &Vm, v: Value) -> Result<String, crate::vm::VmError> {
    if let Value::Obj(h) = v {
        if matches!(vm.heap.get(h), Obj::Str(_)) {
            let raw = vm.str_of(v)?;
            let mut out = String::with_capacity(raw.len() + 2);
            out.push('"');
            for c in raw.chars() {
                match c {
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    _ => out.push(c),
                }
            }
            out.push('"');
            return Ok(out);
        }
    }
    vm.display_value(v)
}

/// Are there unclosed delimiters or strings? (Cheap heuristic for multi-line
/// input: lex and count.)
fn delimiters_open(text: &str) -> bool {
    let lexed = crate::lexer::lex(text);
    let mut depth = 0i32;
    for t in &lexed.tokens {
        match t.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}
