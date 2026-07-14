//! End-to-end language-server test: spawn `fable lsp`, speak JSON-RPC over
//! its stdio, and check diagnostics, hover, and go-to-definition.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};

fn fable_bin() -> &'static str {
    env!("CARGO_BIN_EXE_fable")
}

struct Client {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl Client {
    fn start() -> Client {
        let mut child = Command::new(fable_bin())
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn fable lsp");
        let reader = BufReader::new(child.stdout.take().unwrap());
        Client { child, reader, next_id: 1 }
    }

    fn send(&mut self, body: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: &str) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#
        ));
        id
    }

    fn notify(&mut self, method: &str, params: &str) {
        self.send(&format!(
            r#"{{"jsonrpc":"2.0","method":"{method}","params":{params}}}"#
        ));
    }

    fn read_message(&mut self) -> String {
        let mut len = 0usize;
        loop {
            let mut line = String::new();
            self.reader.read_line(&mut line).expect("read header");
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length:") {
                len = v.trim().parse().unwrap();
            }
        }
        let mut buf = vec![0u8; len];
        self.reader.read_exact(&mut buf).expect("read body");
        String::from_utf8(buf).unwrap()
    }

    /// Read messages until one contains `needle`.
    fn read_until(&mut self, needle: &str) -> String {
        for _ in 0..50 {
            let m = self.read_message();
            if m.contains(needle) {
                return m;
            }
        }
        panic!("no message containing {needle:?}");
    }

    fn shutdown(mut self) {
        self.request("shutdown", "null");
        let _ = self.read_until("\"id\":");
        self.notify("exit", "null");
        let status = self.child.wait().expect("wait");
        assert!(status.success(), "lsp exit status: {status:?}");
    }
}

fn uri_for(name: &str) -> (std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join("fable-lsp-smoke");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    (path.clone(), format!("file://{}", path.display()))
}

#[test]
fn diagnostics_hover_definition() {
    let mut c = Client::start();
    c.request("initialize", r#"{"capabilities":{}}"#);
    let init = c.read_until("fable-lsp");
    assert!(init.contains("hoverProvider"));
    c.notify("initialized", "{}");

    // A document with a type error produces a diagnostic with its code.
    let (path, uri) = uri_for("bad.fable");
    std::fs::write(&path, "").unwrap();
    let bad = r#"let x: Int = \"hi\";"#;
    c.notify(
        "textDocument/didOpen",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri}","languageId":"fable","version":1,"text":"{bad}"}}}}"#
        ),
    );
    let diags = c.read_until("publishDiagnostics");
    assert!(diags.contains("E0301"), "diagnostics: {diags}");

    // Fixing the document clears the diagnostics.
    c.notify(
        "textDocument/didChange",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri}","version":2}},"contentChanges":[{{"text":"let x: Int = 1;"}}]}}"#
        ),
    );
    let diags = c.read_until("publishDiagnostics");
    assert!(diags.contains(r#""diagnostics":[]"#), "diagnostics: {diags}");

    // Hover over a call reports the checked type; definition jumps to the fn.
    let (path2, uri2) = uri_for("good.fable");
    std::fs::write(&path2, "").unwrap();
    let good = r#"fn double(n: Int) -> Int {\n    n * 2\n}\nlet answer = double(21);\n"#;
    c.notify(
        "textDocument/didOpen",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri2}","languageId":"fable","version":1,"text":"{good}"}}}}"#
        ),
    );
    let _ = c.read_until("publishDiagnostics");

    // Position on `double` in the call on line 3: `let answer = double(21);`
    c.request(
        "textDocument/hover",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri2}"}},"position":{{"line":3,"character":14}}}}"#
        ),
    );
    let hover = c.read_until("contents");
    assert!(
        hover.contains("fn(Int) -> Int"),
        "hover: {hover}"
    );

    c.request(
        "textDocument/definition",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri2}"}},"position":{{"line":3,"character":14}}}}"#
        ),
    );
    let def = c.read_until("range");
    // The definition is `double` on line 0, character 3.
    assert!(def.contains(r#""line":0"#), "definition: {def}");
    assert!(def.contains(r#""character":3"#), "definition: {def}");

    // Completion after a dot on a struct value: methods and fields.
    let (path3, uri3) = uri_for("complete.fable");
    std::fs::write(&path3, "").unwrap();
    let src3 = r#"struct Point { x: Float, y: Float }\nimpl Point {\n    fn len(self) -> Float { (self.x * self.x + self.y * self.y).sqrt() }\n}\nlet p = Point { x: 3.0, y: 4.0 };\nlet d = p.len();\n"#;
    c.notify(
        "textDocument/didOpen",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri3}","languageId":"fable","version":1,"text":"{src3}"}}}}"#
        ),
    );
    let _ = c.read_until("publishDiagnostics");
    // Simulate typing `p.` at the end (buffer no longer parses; completion
    // answers from the last good analysis).
    let src4 = format!("{src3}p.");
    c.notify(
        "textDocument/didChange",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri3}","version":2}},"contentChanges":[{{"text":"{src4}"}}]}}"#
        ),
    );
    let _ = c.read_until("publishDiagnostics");
    c.request(
        "textDocument/completion",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri3}"}},"position":{{"line":6,"character":2}}}}"#
        ),
    );
    let comp = c.read_until("label");
    assert!(comp.contains(r#""label":"len""#), "completion: {comp}");
    assert!(comp.contains(r#""label":"x""#), "completion: {comp}");

    // Completion after a module alias dot: std.json's pub members.
    let (path5, uri5) = uri_for("complete_mod.fable");
    std::fs::write(&path5, "").unwrap();
    let src5 = r#"import std.json;\nlet x = 1;\n"#;
    c.notify(
        "textDocument/didOpen",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri5}","languageId":"fable","version":1,"text":"{src5}"}}}}"#
        ),
    );
    let _ = c.read_until("publishDiagnostics");
    let src6 = r#"import std.json;\nlet x = 1;\njson."#;
    c.notify(
        "textDocument/didChange",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri5}","version":2}},"contentChanges":[{{"text":"{src6}"}}]}}"#
        ),
    );
    let _ = c.read_until("publishDiagnostics");
    c.request(
        "textDocument/completion",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri5}"}},"position":{{"line":2,"character":5}}}}"#
        ),
    );
    let comp = c.read_until("label");
    assert!(comp.contains(r#""label":"parse""#), "completion: {comp}");
    assert!(comp.contains(r#""label":"stringify""#), "completion: {comp}");

    c.shutdown();
}
