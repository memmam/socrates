//! Bytecode disassembler (`fable dis file.fable`).

use crate::bytecode::{CompiledProgram, Const, Op};

pub fn disassemble(program: &CompiledProgram) -> String {
    let mut out = String::new();
    use std::fmt::Write;

    if !program.consts.is_empty() {
        let _ = writeln!(out, "; constants");
        for (i, c) in program.consts.iter().enumerate() {
            let shown = match c {
                Const::Int(v) => v.to_string(),
                Const::Float(f) => crate::vm::fmt_float(*f),
                Const::Str(s) => format!("{s:?}"),
            };
            let _ = writeln!(out, ";   [{i}] {shown}");
        }
        out.push('\n');
    }

    for (pi, proto) in program.protos.iter().enumerate() {
        let _ = writeln!(
            out,
            "fn {} (proto {pi}, arity {}, {} upvalues, max locals {})",
            proto.name,
            proto.arity,
            proto.upvals.len(),
            proto.max_locals
        );
        for (i, op) in proto.code.iter().enumerate() {
            let operand = describe(op, i, program);
            let _ = writeln!(out, "  {i:4}  {operand}");
        }
        out.push('\n');
    }
    out
}

fn describe(op: &Op, at: usize, program: &CompiledProgram) -> String {
    let cname = |i: u32| -> String {
        match program.consts.get(i as usize) {
            Some(Const::Int(v)) => v.to_string(),
            Some(Const::Float(f)) => crate::vm::fmt_float(*f),
            Some(Const::Str(s)) => format!("{s:?}"),
            None => "?".into(),
        }
    };
    let target = |off: i32| (at as i64 + 1 + off as i64).to_string();
    match op {
        Op::Const(i) => format!("const       {i} ; {}", cname(*i)),
        Op::Jump(o) => format!("jump        -> {}", target(*o)),
        Op::JumpIfFalse(o) => format!("jmp_false   -> {}", target(*o)),
        Op::JumpIfFalsePeek(o) => format!("jmp_false&  -> {}", target(*o)),
        Op::JumpIfTruePeek(o) => format!("jmp_true&   -> {}", target(*o)),
        Op::ForNext(o) => format!("for_next    done -> {}", target(*o)),
        Op::ForNextRange { off, inclusive } => format!(
            "for_range{}  done -> {}",
            if *inclusive { "=" } else { " " },
            target(*off)
        ),
        Op::GetLocal(s) => format!("get_local   {s}"),
        Op::SetLocal(s) => format!("set_local   {s}"),
        Op::GetGlobal(g) => format!(
            "get_global  {g} ; {}",
            program.global_names.get(*g as usize).map(|s| s.as_str()).unwrap_or("?")
        ),
        Op::SetGlobal(g) => format!(
            "set_global  {g} ; {}",
            program.global_names.get(*g as usize).map(|s| s.as_str()).unwrap_or("?")
        ),
        Op::GetUpvalue(i) => format!("get_upval   {i}"),
        Op::SetUpvalue(i) => format!("set_upval   {i}"),
        Op::PushFn(p) => format!(
            "push_fn     {p} ; {}",
            program.protos.get(*p as usize).map(|f| f.name.as_str()).unwrap_or("?")
        ),
        Op::PushNative(n) => format!("push_native {}", n.name()),
        Op::Closure(p) => format!(
            "closure     {p} ; {}",
            program.protos.get(*p as usize).map(|f| f.name.as_str()).unwrap_or("?")
        ),
        Op::Call(n) => format!("call        {n}"),
        Op::CallFn(p, n) => format!(
            "call_fn     {p} argc={n} ; {}",
            program.protos.get(*p as usize).map(|f| f.name.as_str()).unwrap_or("?")
        ),
        Op::CallNative(nat, n) => format!("call_native {} argc={n}", nat.name()),
        Op::TailCall(n) => format!("tail_call   {n}"),
        Op::TailCallFn(p, n) => format!(
            "tail_callfn {p} argc={n} ; {}",
            program.protos.get(*p as usize).map(|f| f.name.as_str()).unwrap_or("?")
        ),
        Op::MakeVariant { def, variant, arity } => {
            let name = match program.defs.get(*def as usize) {
                Some(crate::bytecode::RtDef::Enum { name, variants }) => format!(
                    "{name}.{}",
                    variants
                        .get(*variant as usize)
                        .map(|(n, _)| n.as_str())
                        .unwrap_or("?")
                ),
                _ => "?".into(),
            };
            format!("make_variant {name} arity={arity}")
        }
        Op::MakeStructEmpty(def) => {
            let name = match program.defs.get(*def as usize) {
                Some(crate::bytecode::RtDef::Struct { name, .. }) => name.clone(),
                _ => "?".into(),
            };
            format!("make_struct {name}")
        }
        other => {
            let s = format!("{other:?}");
            // CamelCase(args) → lower_snake args
            let mut out = String::new();
            for (i, ch) in s.chars().enumerate() {
                if ch.is_ascii_uppercase() {
                    if i > 0 {
                        out.push('_');
                    }
                    out.push(ch.to_ascii_lowercase());
                } else if ch == '(' {
                    out.push(' ');
                } else if ch == ')' {
                } else {
                    out.push(ch);
                }
            }
            out
        }
    }
}
