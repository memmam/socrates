//! The Fable language server: JSON-RPC over stdio (Content-Length framing),
//! zero dependencies like everything else.
//!
//! Capabilities: full-text document sync with diagnostics on open/change,
//! hover (the checked type of the expression under the cursor), and
//! go-to-definition for variables, globals, functions, and methods. Analysis
//! runs the ordinary loader/checker pipeline with the unsaved buffer
//! overlaid, so diagnostics match `fable check` exactly.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::ast::*;
use crate::builtins::{Native, Recv};
use crate::check::{Checker, Res};
use crate::diag::Diagnostic;
use crate::jsonlite::{parse as jparse, J};
use crate::modules::{self, ModuleUnit};
use crate::span::{NodeId, Span};

pub fn run_lsp() -> i32 {
    let stdin = std::io::stdin();
    let mut reader = std::io::BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut server = Server {
        out: Box::new(stdout.lock()),
        docs: HashMap::new(),
        shutdown_seen: false,
    };
    loop {
        let Some(msg) = read_message(&mut reader) else {
            // Client hung up without `exit`.
            return if server.shutdown_seen { 0 } else { 1 };
        };
        let Ok(v) = jparse(&msg) else { continue };
        let method = v.get("method").and_then(J::as_str).unwrap_or("");
        if method == "exit" {
            return if server.shutdown_seen { 0 } else { 1 };
        }
        server.dispatch(&v);
    }
}

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

fn read_message(reader: &mut impl BufRead) -> Option<String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok();
        }
    }
    let n = content_length?;
    let mut buf = vec![0u8; n];
    reader.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

fn write_message(out: &mut dyn Write, v: &J) {
    let body = v.to_string();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = out.flush();
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

struct Analysis {
    units: Vec<ModuleUnit>,
    checker: Checker,
    /// Diagnostics per unit index (same order as `units`).
    diags: Vec<Vec<Diagnostic>>,
    /// A whole-load failure (unreadable import, cycle, parse error): the
    /// source it belongs to plus its diagnostics.
    load_error: Option<(String, Vec<Diagnostic>)>,
}

struct Doc {
    text: String,
    analysis: Option<Analysis>,
    /// The most recent analysis whose load succeeded (its tree and side
    /// tables back completion while the buffer is mid-edit), with the text
    /// it was computed from (span math must use that text).
    last_good: Option<(Analysis, String)>,
}

struct Server<'a> {
    out: Box<dyn Write + 'a>,
    docs: HashMap<String, Doc>,
    shutdown_seen: bool,
}

impl Server<'_> {
    fn respond(&mut self, id: &J, result: J) {
        let msg = J::obj(vec![
            ("jsonrpc", J::str("2.0")),
            ("id", id.clone()),
            ("result", result),
        ]);
        write_message(&mut self.out, &msg);
    }

    fn notify(&mut self, method: &str, params: J) {
        let msg = J::obj(vec![
            ("jsonrpc", J::str("2.0")),
            ("method", J::str(method)),
            ("params", params),
        ]);
        write_message(&mut self.out, &msg);
    }

    fn dispatch(&mut self, v: &J) {
        let method = v.get("method").and_then(J::as_str).unwrap_or("").to_string();
        let id = v.get("id").cloned();
        let params = v.get("params").cloned().unwrap_or(J::Null);
        match method.as_str() {
            "initialize" => {
                let caps = J::obj(vec![
                    ("textDocumentSync", J::Num(1.0)), // full
                    ("hoverProvider", J::Bool(true)),
                    ("definitionProvider", J::Bool(true)),
                    (
                        "completionProvider",
                        J::obj(vec![("triggerCharacters", J::Arr(vec![J::str(".")]))]),
                    ),
                ]);
                let result = J::obj(vec![
                    ("capabilities", caps),
                    (
                        "serverInfo",
                        J::obj(vec![
                            ("name", J::str("fable-lsp")),
                            ("version", J::str(env!("CARGO_PKG_VERSION"))),
                        ]),
                    ),
                ]);
                if let Some(id) = &id {
                    self.respond(id, result);
                }
            }
            "shutdown" => {
                self.shutdown_seen = true;
                if let Some(id) = &id {
                    self.respond(id, J::Null);
                }
            }
            "textDocument/didOpen" => {
                let (Some(uri), Some(text)) = (
                    params.get("textDocument").and_then(|t| t.get("uri")).and_then(J::as_str),
                    params.get("textDocument").and_then(|t| t.get("text")).and_then(J::as_str),
                ) else {
                    return;
                };
                let uri = uri.to_string();
                self.update_doc(&uri, text.to_string());
            }
            "textDocument/didChange" => {
                let Some(uri) = params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(J::as_str)
                else {
                    return;
                };
                let uri = uri.to_string();
                let Some(text) = params
                    .get("contentChanges")
                    .and_then(J::as_arr)
                    .and_then(|a| a.first())
                    .and_then(|c| c.get("text"))
                    .and_then(J::as_str)
                else {
                    return;
                };
                self.update_doc(&uri, text.to_string());
            }
            "textDocument/didClose" => {
                if let Some(uri) = params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .and_then(J::as_str)
                {
                    let uri = uri.to_string();
                    self.docs.remove(&uri);
                    self.notify(
                        "textDocument/publishDiagnostics",
                        J::obj(vec![("uri", J::str(uri)), ("diagnostics", J::Arr(vec![]))]),
                    );
                }
            }
            "textDocument/hover" => {
                let result = self.hover(&params).unwrap_or(J::Null);
                if let Some(id) = &id {
                    self.respond(id, result);
                }
            }
            "textDocument/definition" => {
                let result = self.definition(&params).unwrap_or(J::Null);
                if let Some(id) = &id {
                    self.respond(id, result);
                }
            }
            "textDocument/completion" => {
                let result = self
                    .completion(&params)
                    .unwrap_or_else(|| J::Arr(vec![]));
                if let Some(id) = &id {
                    self.respond(id, result);
                }
            }
            _ => {
                // Unknown request: answer null so clients don't hang.
                if let Some(id) = &id {
                    self.respond(id, J::Null);
                }
            }
        }
    }

    fn update_doc(&mut self, uri: &str, text: String) {
        let analysis = uri_to_path(uri).map(|path| analyze(&path, &text));
        let diags_json = analysis
            .as_ref()
            .map(|a| diagnostics_json(a, &text))
            .unwrap_or_default();
        let mut last_good = self.docs.remove(uri).and_then(|d| d.last_good);
        if let Some(a) = &analysis {
            if !a.units.is_empty() {
                last_good = Some((
                    Analysis {
                        units: a.units.clone(),
                        checker: a.checker.clone(),
                        diags: Vec::new(),
                        load_error: None,
                    },
                    text.clone(),
                ));
            }
        }
        self.docs.insert(uri.to_string(), Doc { text, analysis, last_good });
        self.notify(
            "textDocument/publishDiagnostics",
            J::obj(vec![
                ("uri", J::str(uri)),
                ("diagnostics", J::Arr(diags_json)),
            ]),
        );
    }

    fn doc_at<'d>(&'d self, params: &J) -> Option<(&'d Doc, u32)> {
        let uri = params
            .get("textDocument")
            .and_then(|t| t.get("uri"))
            .and_then(J::as_str)?;
        let doc = self.docs.get(uri)?;
        let pos = params.get("position")?;
        let line = pos.get("line")?.as_f64()? as usize;
        let character = pos.get("character")?.as_f64()? as usize;
        let byte = lsp_pos_to_byte(&doc.text, line, character)?;
        Some((doc, byte))
    }

    fn hover(&self, params: &J) -> Option<J> {
        let (doc, byte) = self.doc_at(params)?;
        let a = doc.analysis.as_ref()?;
        let root = a.units.last()?;
        let (id, span) = node_at(&root.program, byte)?;
        let ty = a.checker.types.get(&id)?;
        let shown = a.checker.display_type_public(ty);
        let contents = J::obj(vec![
            ("kind", J::str("markdown")),
            ("value", J::str(format!("```fable\n{shown}\n```"))),
        ]);
        Some(J::obj(vec![
            ("contents", contents),
            ("range", range_json(&doc.text, span)),
        ]))
    }

    fn definition(&self, params: &J) -> Option<J> {
        let uri = params
            .get("textDocument")
            .and_then(|t| t.get("uri"))
            .and_then(J::as_str)?
            .to_string();
        let (doc, byte) = self.doc_at(params)?;
        let a = doc.analysis.as_ref()?;
        let root = a.units.last()?;
        let (id, _) = node_at(&root.program, byte)?;
        let (target_name, span) = match a.checker.res.get(&id)? {
            Res::Local(i) => {
                let info = a.checker.locals.get(*i as usize)?;
                (None, info.span)
            }
            Res::Global(i) => {
                let info = a.checker.globals.get(*i as usize)?;
                (Some(info.name.clone()), info.span)
            }
            Res::Fn(i) | Res::ModuleFn(i) => {
                let info = a.checker.fns.get(*i as usize)?;
                (Some(info.name.clone()), info.span)
            }
            _ => return None,
        };
        // Locals are always in the current document; named items live in the
        // module their stored (prefixed) name says.
        let (target_uri, target_text) = match target_name {
            None => (uri, doc.text.clone()),
            Some(name) => {
                let prefix = match name.rfind('.') {
                    Some(i) => &name[..i],
                    None => "",
                };
                let unit = a.units.iter().find(|u| u.prefix == prefix)?;
                if unit.prefix == root.prefix {
                    (uri, doc.text.clone())
                } else {
                    if unit.source.name.starts_with('<') {
                        return None; // embedded std module: no file to open
                    }
                    (path_to_uri(Path::new(&unit.source.name)), unit.source.text.clone())
                }
            }
        };
        Some(J::obj(vec![
            ("uri", J::str(target_uri)),
            ("range", range_json(&target_text, span)),
        ]))
    }
}

impl Server<'_> {
    /// textDocument/completion: identify the context from the CURRENT text
    /// (an identifier stem, possibly after a `.`), then answer from the last
    /// analysis that produced a tree.
    fn completion(&self, params: &J) -> Option<J> {
        let uri = params
            .get("textDocument")
            .and_then(|t| t.get("uri"))
            .and_then(J::as_str)?;
        let doc = self.docs.get(uri)?;
        let pos = params.get("position")?;
        let line = pos.get("line")?.as_f64()? as usize;
        let character = pos.get("character")?.as_f64()? as usize;
        let byte = lsp_pos_to_byte(&doc.text, line, character)? as usize;

        // Scan back over the identifier being typed, then check for a dot.
        let bytes = doc.text.as_bytes();
        let mut stem_start = byte;
        while stem_start > 0
            && (bytes[stem_start - 1].is_ascii_alphanumeric() || bytes[stem_start - 1] == b'_')
        {
            stem_start -= 1;
        }
        let dot = stem_start > 0 && bytes[stem_start - 1] == b'.';

        let (a, a_text) = match &doc.last_good {
            Some((a, t)) => (a, t.as_str()),
            None => return Some(J::Arr(completion_top_level(None))),
        };

        let mut items = Vec::new();
        if dot {
            // The receiver chain is the maximal ident/dot/() run before the dot.
            let recv_end = stem_start - 1;
            let mut recv_start = recv_end;
            while recv_start > 0 {
                let b = bytes[recv_start - 1];
                if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b')' || b == b'(' {
                    recv_start -= 1;
                } else {
                    break;
                }
            }
            let chain = &doc.text[recv_start..recv_end];

            // Import alias → module members.
            let root = a.units.last()?;
            if let Some(key) = root.imports.get(chain) {
                items.extend(completion_module_members(a, key));
            } else if Native::is_namespace(chain) {
                for name in Native::namespace_members(chain) {
                    items.push(completion_item(name, 3, ""));
                }
            } else {
                // An expression receiver: find a node in the last good tree
                // whose source text matches the chain, and use its type.
                if let Some(ty) = type_of_source_chain(a, a_text, chain) {
                    items.extend(completion_for_type(a, &ty));
                }
            }
        } else {
            items = completion_top_level(Some(a));
        }
        Some(J::Arr(items))
    }
}

fn completion_item(label: &str, kind: u32, detail: &str) -> J {
    let mut fields = vec![("label", J::str(label)), ("kind", J::Num(kind as f64))];
    if !detail.is_empty() {
        fields.push(("detail", J::str(detail)));
    }
    J::obj(fields)
}

/// Members of an imported module: pub fns, globals, and types.
fn completion_module_members(a: &Analysis, key: &str) -> Vec<J> {
    let prefix = format!("{key}.");
    let mut items = Vec::new();
    for f in &a.checker.fns {
        if f.is_pub && f.name.starts_with(&prefix) {
            let bare = &f.name[prefix.len()..];
            if !bare.contains('.') {
                items.push(completion_item(bare, 3, "fn"));
            }
        }
    }
    for g in &a.checker.globals {
        if g.is_pub && g.name.starts_with(&prefix) {
            let bare = &g.name[prefix.len()..];
            if !bare.contains('.') {
                items.push(completion_item(bare, 6, &a.checker.display_type_public(&g.ty)));
            }
        }
    }
    for def in 0..a.checker.defs.types.len() as u32 {
        let td = a.checker.defs.get(def);
        let (name, is_pub) = match td {
            crate::types::TypeDef::Struct(s) => (&s.name, s.is_pub),
            crate::types::TypeDef::Enum(e) => (&e.name, e.is_pub),
        };
        if is_pub && name.starts_with(&prefix) {
            let bare = &name[prefix.len()..];
            if !bare.contains('.') {
                items.push(completion_item(bare, 7, "type"));
            }
        }
    }
    items
}

/// Find the type of an expression in the last good tree whose SOURCE TEXT
/// equals `chain` (e.g. "self.pos", "xs"). Prefers the smallest such node.
fn type_of_source_chain(a: &Analysis, text: &str, chain: &str) -> Option<crate::types::Type> {
    if chain.is_empty() {
        return None;
    }
    let root = a.units.last()?;
    let mut found: Option<(u32, crate::types::Type)> = None;
    let mut consider = |id: NodeId, span: Span| {
        let slice = text.get(span.start as usize..span.end as usize);
        if slice == Some(chain) {
            if let Some(ty) = a.checker.types.get(&id) {
                let size = span.end - span.start;
                if found.as_ref().is_none_or(|(s, _)| size <= *s) {
                    found = Some((size, ty.clone()));
                }
            }
        }
    };
    for stmt in &root.program.stmts {
        walk_stmt(stmt, &mut consider);
    }
    found.map(|(_, t)| t)
}

/// Completions for a value of a known type: builtin methods, user methods,
/// struct fields, tuple indices.
fn completion_for_type(a: &Analysis, ty: &crate::types::Type) -> Vec<J> {
    use crate::types::{Type, TypeDef, OPTION_DEF, RESULT_DEF};
    let mut items = Vec::new();
    let resolved = a.checker.uni.zonk(ty);
    let recv = match &resolved {
        Type::Int => Some(Recv::Int),
        Type::Float => Some(Recv::Float),
        Type::Str => Some(Recv::Str),
        Type::Range => Some(Recv::Range),
        Type::List(_) => Some(Recv::List),
        Type::Map(_, _) => Some(Recv::Map),
        Type::Named(d, _) if *d == OPTION_DEF => Some(Recv::Option_),
        Type::Named(d, _) if *d == RESULT_DEF => Some(Recv::Result_),
        _ => None,
    };
    if let Some(recv) = recv {
        for name in Native::methods_of(recv) {
            items.push(completion_item(name, 2, "method"));
        }
    }
    if let Type::Named(def, _) = &resolved {
        for (name, is_pub) in a.checker.methods_on(*def) {
            let detail = if is_pub { "method" } else { "method (private)" };
            items.push(completion_item(&name, 2, detail));
        }
        if let TypeDef::Struct(s) = a.checker.defs.get(*def) {
            for (fname, fty) in &s.fields {
                items.push(completion_item(fname, 5, &a.checker.display_type_public(fty)));
            }
        }
    }
    if let Type::Tuple(ts) = &resolved {
        for (i, t) in ts.iter().enumerate() {
            items.push(completion_item(
                &i.to_string(),
                5,
                &a.checker.display_type_public(t),
            ));
        }
    }
    // The universal method.
    items.push(completion_item("to_string", 2, "method"));
    items
}

/// Bare-identifier completions: top-level names, aliases, namespaces,
/// prelude constructors, keywords.
fn completion_top_level(a: Option<&Analysis>) -> Vec<J> {
    let mut items = Vec::new();
    if let Some(a) = a {
        if let Some(root) = a.units.last() {
            for f in &a.checker.fns {
                if !f.name.contains('.') {
                    items.push(completion_item(&f.name, 3, "fn"));
                }
            }
            for g in &a.checker.globals {
                if !g.name.contains('.') {
                    items.push(completion_item(&g.name, 6, &a.checker.display_type_public(&g.ty)));
                }
            }
            for def in 0..a.checker.defs.types.len() as u32 {
                let name = a.checker.defs.get(def).name();
                if !name.contains('.') {
                    items.push(completion_item(name, 7, "type"));
                }
            }
            for alias in root.imports.keys() {
                items.push(completion_item(alias, 9, "module"));
            }
        }
    }
    for ns in ["math", "fs", "os"] {
        items.push(completion_item(ns, 9, "namespace"));
    }
    for ctor in ["Some", "None", "Ok", "Err", "true", "false"] {
        items.push(completion_item(ctor, 4, ""));
    }
    for kw in [
        "let", "mut", "pub", "fn", "struct", "enum", "impl", "import", "match", "if",
        "else", "while", "for", "in", "return", "break", "continue",
    ] {
        items.push(completion_item(kw, 14, ""));
    }
    items
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

fn analyze(path: &Path, text: &str) -> Analysis {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut overlay = HashMap::new();
    overlay.insert(canon, text.to_string());
    let search: Vec<PathBuf> = std::env::var("FABLE_PATH")
        .ok()
        .map(|v| v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect())
        .unwrap_or_default();
    match modules::load_modules_overlay(path, &search, &overlay) {
        Err((source, diags)) => Analysis {
            units: Vec::new(),
            checker: Checker::new(),
            diags: Vec::new(),
            load_error: Some((source.name.clone(), diags)),
        },
        Ok(units) => {
            let mut checker = Checker::new();
            let mut per_unit = Vec::new();
            for unit in &units {
                checker.check_module(&unit.program, &unit.prefix, unit.imports.clone());
                per_unit.push(checker.take_diags());
            }
            Analysis { units, checker, diags: per_unit, load_error: None }
        }
    }
}

/// The diagnostics to publish for the open document: its own, plus one
/// summary entry when a dependency failed.
fn diagnostics_json(a: &Analysis, text: &str) -> Vec<J> {
    let mut out = Vec::new();
    if let Some((source_name, diags)) = &a.load_error {
        let root_failed = a.units.is_empty() && !source_name.starts_with('<');
        // If the failure is in the document itself its spans apply directly;
        // otherwise summarize at the top of the file.
        let in_doc = root_failed && diags.iter().any(|d| d.primary_span().is_some());
        if in_doc {
            for d in diags {
                out.push(diag_json(d, text));
            }
        } else {
            let first = diags.first();
            let msg = match first {
                Some(d) => format!("error in `{source_name}`: {}", d.message),
                None => format!("error in `{source_name}`"),
            };
            out.push(J::obj(vec![
                ("range", range_json(text, Span::new(0, 0))),
                ("severity", J::Num(1.0)),
                ("source", J::str("fable")),
                ("message", J::str(msg)),
            ]));
        }
        return out;
    }
    let root_idx = a.units.len().saturating_sub(1);
    for (i, diags) in a.diags.iter().enumerate() {
        for d in diags {
            if i == root_idx {
                out.push(diag_json(d, text));
            } else {
                // A dependency has an error: summarize on the document.
                if d.is_error() {
                    out.push(J::obj(vec![
                        ("range", range_json(text, Span::new(0, 0))),
                        ("severity", J::Num(1.0)),
                        ("source", J::str("fable")),
                        (
                            "message",
                            J::str(format!(
                                "error in imported module `{}`: {}",
                                a.units[i].source.name, d.message
                            )),
                        ),
                    ]));
                }
            }
        }
    }
    out
}

fn diag_json(d: &Diagnostic, text: &str) -> J {
    let span = d.primary_span().unwrap_or(Span::new(0, 0));
    let mut message = d.message.clone();
    for n in &d.notes {
        message.push_str("\nnote: ");
        message.push_str(n);
    }
    J::obj(vec![
        ("range", range_json(text, span)),
        ("severity", J::Num(if d.is_error() { 1.0 } else { 2.0 })),
        ("code", J::str(d.code)),
        ("source", J::str("fable")),
        ("message", J::str(message)),
    ])
}

// ---------------------------------------------------------------------------
// Positions (LSP speaks UTF-16 line/character)
// ---------------------------------------------------------------------------

fn lsp_pos_to_byte(text: &str, line: usize, character: usize) -> Option<u32> {
    let mut cur = 0usize;
    let mut offset = 0usize;
    for l in text.split_inclusive('\n') {
        if cur == line {
            let mut units = 0usize;
            for (i, c) in l.char_indices() {
                if units >= character {
                    return Some((offset + i) as u32);
                }
                units += c.len_utf16();
            }
            return Some((offset + l.len()) as u32);
        }
        offset += l.len();
        cur += 1;
    }
    if cur == line {
        return Some(offset as u32);
    }
    None
}

fn byte_to_lsp_pos(text: &str, byte: u32) -> (usize, usize) {
    let byte = (byte as usize).min(text.len());
    let mut line = 0usize;
    let mut line_start = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if i >= byte {
            break;
        }
        if b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    let character = text[line_start..byte]
        .chars()
        .map(char::len_utf16)
        .sum();
    (line, character)
}

fn range_json(text: &str, span: Span) -> J {
    let (sl, sc) = byte_to_lsp_pos(text, span.start);
    let (el, ec) = byte_to_lsp_pos(text, span.end);
    J::obj(vec![
        (
            "start",
            J::obj(vec![("line", J::Num(sl as f64)), ("character", J::Num(sc as f64))]),
        ),
        (
            "end",
            J::obj(vec![("line", J::Num(el as f64)), ("character", J::Num(ec as f64))]),
        ),
    ])
}

// ---------------------------------------------------------------------------
// URI ↔ path
// ---------------------------------------------------------------------------

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // Strip an authority component if present (file://host/... is rare).
    let path = if let Some(idx) = rest.find('/') { &rest[idx..] } else { rest };
    Some(PathBuf::from(percent_decode(path)))
}

fn path_to_uri(path: &Path) -> String {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut out = String::from("file://");
    for c in canon.display().to_string().chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '/' | '.' | '-' | '_' | '~' => out.push(c),
            c => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// Finding the node under the cursor
// ---------------------------------------------------------------------------

/// The smallest expression (or parameter) whose span contains `byte`.
pub fn node_at(program: &Program, byte: u32) -> Option<(NodeId, Span)> {
    let mut best: Option<(NodeId, Span)> = None;
    let mut consider = |id: NodeId, span: Span| {
        if span.start <= byte && byte < span.end {
            let better = match best {
                None => true,
                Some((_, b)) => span.end - span.start <= b.end - b.start,
            };
            if better {
                best = Some((id, span));
            }
        }
    };
    for stmt in &program.stmts {
        walk_stmt(stmt, &mut consider);
    }
    best
}

fn walk_stmt(stmt: &Stmt, f: &mut impl FnMut(NodeId, Span)) {
    match &stmt.kind {
        StmtKind::Fn(d) => walk_fn(d, f),
        StmtKind::Impl(im) => {
            for m in &im.methods {
                walk_fn(m, f);
            }
        }
        StmtKind::Struct(_) | StmtKind::Enum(_) | StmtKind::Import { .. } => {}
        StmtKind::Let { pattern, init, .. } => {
            walk_pattern(pattern, f);
            walk_expr(init, f);
        }
        StmtKind::Assign { target, value, .. } => {
            walk_expr(target, f);
            walk_expr(value, f);
        }
        StmtKind::Expr { expr, .. } => walk_expr(expr, f),
        StmtKind::While { cond, body } => {
            walk_expr(cond, f);
            walk_block(body, f);
        }
        StmtKind::For { pattern, iter, body } => {
            walk_pattern(pattern, f);
            walk_expr(iter, f);
            walk_block(body, f);
        }
        StmtKind::Return(v) => {
            if let Some(v) = v {
                walk_expr(v, f);
            }
        }
        StmtKind::Break | StmtKind::Continue => {}
    }
}

fn walk_fn(d: &FnDecl, f: &mut impl FnMut(NodeId, Span)) {
    for p in &d.params {
        f(p.id, p.name.span);
    }
    walk_block(&d.body, f);
}

fn walk_block(b: &Block, f: &mut impl FnMut(NodeId, Span)) {
    for stmt in &b.stmts {
        walk_stmt(stmt, f);
    }
}

fn walk_pattern(p: &Pattern, f: &mut impl FnMut(NodeId, Span)) {
    f(p.id, p.span);
    match &p.kind {
        PatternKind::Tuple(items) | PatternKind::Or(items) => {
            for q in items {
                walk_pattern(q, f);
            }
        }
        PatternKind::Variant { fields, .. } => {
            for q in fields {
                walk_pattern(q, f);
            }
        }
        PatternKind::Struct { fields, .. } => {
            for (_, q) in fields {
                walk_pattern(q, f);
            }
        }
        _ => {}
    }
}

fn walk_expr(e: &Expr, f: &mut impl FnMut(NodeId, Span)) {
    f(e.id, e.span);
    match &e.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Unit
        | ExprKind::Var(_) => {}
        ExprKind::StringInterp { exprs, .. } => {
            for x in exprs {
                walk_expr(x, f);
            }
        }
        ExprKind::Field { base, .. } => walk_expr(base, f),
        ExprKind::Call { callee, args } => {
            walk_expr(callee, f);
            for a in args {
                walk_expr(a, f);
            }
        }
        ExprKind::MethodCall { recv, args, .. } => {
            walk_expr(recv, f);
            for a in args {
                walk_expr(a, f);
            }
        }
        ExprKind::Unary { expr, .. } | ExprKind::Try(expr) => walk_expr(expr, f),
        ExprKind::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, f);
            walk_expr(rhs, f);
        }
        ExprKind::Index { base, index } => {
            walk_expr(base, f);
            walk_expr(index, f);
        }
        ExprKind::List(items) | ExprKind::Tuple(items) => {
            for x in items {
                walk_expr(x, f);
            }
        }
        ExprKind::MapLit(entries) => {
            for (k, v) in entries {
                walk_expr(k, f);
                walk_expr(v, f);
            }
        }
        ExprKind::Range { lo, hi, .. } => {
            walk_expr(lo, f);
            walk_expr(hi, f);
        }
        ExprKind::StructLit { fields, .. } => {
            for (_, v) in fields {
                walk_expr(v, f);
            }
        }
        ExprKind::Lambda { params, body, .. } => {
            for p in params {
                f(p.id, p.name.span);
            }
            walk_expr(body, f);
        }
        ExprKind::If { cond, then, els } => {
            walk_expr(cond, f);
            walk_block(then, f);
            if let Some(els) = els {
                walk_expr(els, f);
            }
        }
        ExprKind::Block(b) => walk_block(b, f),
        ExprKind::Match { scrutinee, arms } => {
            walk_expr(scrutinee, f);
            for arm in arms {
                walk_pattern(&arm.pattern, f);
                if let Some(g) = &arm.guard {
                    walk_expr(g, f);
                }
                walk_expr(&arm.body, f);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_mapping_utf16() {
        let text = "let a = 1;\nlet é😀 = 2;\n";
        // 'é' is 1 utf16 unit, 2 bytes; '😀' is 2 utf16 units, 4 bytes.
        let byte = lsp_pos_to_byte(text, 1, 4).unwrap(); // after "let " on line 1
        assert_eq!(&text[byte as usize..byte as usize + 2], "é");
        let (l, c) = byte_to_lsp_pos(text, byte);
        assert_eq!((l, c), (1, 4));
        // Past é (1 unit) and 😀 (2 units): character 7 is the space before `=`.
        let byte2 = lsp_pos_to_byte(text, 1, 7).unwrap();
        assert_eq!(&text[byte2 as usize..byte2 as usize + 1], " ");
    }

    #[test]
    fn framing_roundtrip() {
        let payload = r#"{"jsonrpc":"2.0","method":"exit"}"#;
        let framed = format!("Content-Length: {}\r\n\r\n{payload}", payload.len());
        let mut reader = std::io::BufReader::new(framed.as_bytes());
        assert_eq!(read_message(&mut reader).unwrap(), payload);
        assert!(read_message(&mut reader).is_none());
    }

    #[test]
    fn uri_conversions() {
        assert_eq!(
            uri_to_path("file:///a/b%20c/d.fable").unwrap(),
            PathBuf::from("/a/b c/d.fable")
        );
    }
}
