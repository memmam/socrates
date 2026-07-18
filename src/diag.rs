//! Diagnostics: structured errors/warnings and a Rust-style terminal renderer.
//!
//! Rendered format:
//! ```text
//! error[E0301]: type mismatch
//!   --> examples/foo.soc:3:9
//!    |
//!  3 |     let x: Int = "hi";
//!    |            ---   ^^^^ expected `Int`, found `String`
//!    |            expected due to this annotation
//! ```

use crate::source::Source;
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// A label pointing at a span, with an optional message. The primary label is
/// underlined with `^^^`, secondary labels with `---`.
#[derive(Debug, Clone)]
pub struct Label {
    pub span: Span,
    pub message: String,
    pub primary: bool,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable code such as `E0301` or `W0101`.
    pub code: &'static str,
    pub message: String,
    pub labels: Vec<Label>,
    /// Free-standing notes rendered after the snippet (`note: ...`).
    pub notes: Vec<String>,
}

impl Diagnostic {
    /// The primary label's span, if any (the first label attached).
    pub fn primary_span(&self) -> Option<crate::span::Span> {
        self.labels.first().map(|l| l.span)
    }

    pub fn error(code: &'static str, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            code,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
        }
    }

    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Diagnostic {
        self.labels.push(Label { span, message: message.into(), primary: true });
        self
    }

    pub fn with_secondary(mut self, span: Span, message: impl Into<String>) -> Diagnostic {
        self.labels.push(Label { span, message: message.into(), primary: false });
        self
    }

    pub fn with_note(mut self, message: impl Into<String>) -> Diagnostic {
        self.notes.push(message.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// ANSI style codes, disabled when not writing to a terminal.
struct Style {
    enabled: bool,
}

impl Style {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.enabled {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
    fn red_bold(&self, t: &str) -> String {
        self.paint("1;31", t)
    }
    fn yellow_bold(&self, t: &str) -> String {
        self.paint("1;33", t)
    }
    fn blue_bold(&self, t: &str) -> String {
        self.paint("1;34", t)
    }
    fn bold(&self, t: &str) -> String {
        self.paint("1", t)
    }
}

/// Render a batch of diagnostics against a source file into a string.
/// Error cascades are capped: at most 50 diagnostics render, followed by a
/// one-line summary of how many were suppressed.
pub fn render(diags: &[Diagnostic], source: &Source, color: bool) -> String {
    const MAX_RENDERED: usize = 50;
    let mut out = String::new();
    for (i, d) in diags.iter().take(MAX_RENDERED).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        render_one(&mut out, d, source, color);
    }
    if diags.len() > MAX_RENDERED {
        let n_err = diags.iter().skip(MAX_RENDERED).filter(|d| d.is_error()).count();
        let n_warn = diags.len() - MAX_RENDERED - n_err;
        out.push_str(&format!(
            "\n... and {n_err} more error(s) and {n_warn} more warning(s) not shown\n"
        ));
    }
    out
}

fn render_one(out: &mut String, d: &Diagnostic, source: &Source, color: bool) {
    use std::fmt::Write;
    let st = Style { enabled: color };

    let (head, head_colored) = match d.severity {
        Severity::Error => (format!("error[{}]", d.code), st.red_bold(&format!("error[{}]", d.code))),
        Severity::Warning => {
            (format!("warning[{}]", d.code), st.yellow_bold(&format!("warning[{}]", d.code)))
        }
    };
    let _ = head;
    let _ = writeln!(out, "{}{} {}", head_colored, st.bold(":"), st.bold(&d.message));

    // Group labels by line, primary first for the arrow position.
    let primary = d.labels.iter().find(|l| l.primary).or(d.labels.first());
    if let Some(primary) = primary {
        let lc = source.line_col(primary.span.start);
        let _ = writeln!(
            out,
            "  {} {}:{}:{}",
            st.blue_bold("-->"),
            source.name,
            lc.line,
            lc.col
        );

        // Collect the distinct lines that labels touch, in order.
        let mut lines: Vec<u32> = d
            .labels
            .iter()
            .map(|l| source.line_col(l.span.start).line)
            .collect();
        lines.sort_unstable();
        lines.dedup();

        let gutter_w = lines.iter().map(|l| l.to_string().len()).max().unwrap_or(1);
        let bar = st.blue_bold("|");
        let _ = writeln!(out, "{:w$} {}", "", bar, w = gutter_w + 1);

        for (li, &line) in lines.iter().enumerate() {
            if li > 0 {
                // Ellipsis between non-adjacent labeled lines.
                if line > lines[li - 1] + 1 {
                    let _ = writeln!(out, "{:w$}{}", "", st.blue_bold("..."), w = gutter_w);
                }
            }
            let full_text = source.line_text(line);
            // Pathological single-line programs shouldn't produce megabyte
            // diagnostics: render a bounded window of very long lines.
            const MAX_LINE: usize = 240;
            let truncated = full_text.chars().count() > MAX_LINE;
            let text: String = if truncated {
                let head: String = full_text.chars().take(MAX_LINE).collect();
                format!("{head}…")
            } else {
                full_text.to_string()
            };
            let _ = writeln!(
                out,
                "{:>w$} {} {}",
                st.blue_bold(&line.to_string()),
                bar,
                text,
                w = if color { gutter_w + 9 } else { gutter_w } // account for ANSI bytes
            );

            // Underline row(s) for labels on this line.
            let mut labels_here: Vec<&Label> = d
                .labels
                .iter()
                .filter(|l| source.line_col(l.span.start).line == line)
                .collect();
            labels_here.sort_by_key(|l| l.span.start);

            let line_start = source.line_col(primary.span.start); // placeholder to appease borrow
            let _ = line_start;

            let mut underline = String::new();
            let mut cursor_col = 0usize;
            let mut trailing_msg = String::new();
            let mut trailing_primary = false;
            for l in &labels_here {
                let start = source.line_col(l.span.start).col as usize - 1;
                let end_off = l.span.end.max(l.span.start + 1);
                let end = {
                    let e = source.line_col(end_off);
                    if e.line == line {
                        (e.col as usize - 1).max(start + 1)
                    } else {
                        text.chars().count().max(start + 1)
                    }
                };
                if start < cursor_col {
                    continue; // overlapping labels: keep the first
                }
                // Clamp labels into the rendered window of truncated lines.
                let start = start.min(MAX_LINE);
                let end = end.min(MAX_LINE + 1).max(start + 1);
                if start < cursor_col {
                    continue;
                }
                underline.push_str(&" ".repeat(start - cursor_col));
                let ch = if l.primary { "^" } else { "-" };
                let marks = ch.repeat(end - start);
                underline.push_str(&if l.primary {
                    match d.severity {
                        Severity::Error => st.red_bold(&marks),
                        Severity::Warning => st.yellow_bold(&marks),
                    }
                } else {
                    st.blue_bold(&marks)
                });
                cursor_col = end;
                // The message of the last label on the line rides at the end.
                trailing_msg = l.message.clone();
                trailing_primary = l.primary;
            }
            if !trailing_msg.is_empty() {
                underline.push(' ');
                underline.push_str(&if trailing_primary {
                    match d.severity {
                        Severity::Error => st.red_bold(&trailing_msg),
                        Severity::Warning => st.yellow_bold(&trailing_msg),
                    }
                } else {
                    st.blue_bold(&trailing_msg)
                });
            }
            if !underline.trim().is_empty() {
                let _ = writeln!(out, "{:w$} {} {}", "", bar, underline, w = gutter_w + 1);
            }
            // Messages of earlier labels on this line (all but the last) get
            // their own rows, aligned under their spans.
            if labels_here.len() > 1 {
                for l in &labels_here[..labels_here.len() - 1] {
                    if l.message.is_empty() {
                        continue;
                    }
                    let start = source.line_col(l.span.start).col as usize - 1;
                    let _ = writeln!(
                        out,
                        "{:w$} {} {}{}",
                        "",
                        bar,
                        " ".repeat(start),
                        st.blue_bold(&l.message),
                        w = gutter_w + 1
                    );
                }
            }
        }
    }

    for note in &d.notes {
        let _ = writeln!(out, "  {} {}", st.blue_bold("note:"), note);
    }
}

/// Convenience: true if any diagnostic in the batch is an error.
pub fn has_errors(diags: &[Diagnostic]) -> bool {
    diags.iter().any(|d| d.is_error())
}
