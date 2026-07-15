//! Formatter invariants, enforced over the whole example + spec corpus:
//! 1. formatting a valid program yields a valid program,
//! 2. formatting is idempotent (fmt(fmt(x)) == fmt(x)),
//! 3. the formatted program behaves identically (same stdout / outcome).
//!
//! Plus golden tests for the line-width-aware layouts (100 columns by
//! default): every golden asserts the exact formatted output, idempotency,
//! and that no emitted line exceeds the width unless a single unbreakable
//! token does.

use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "fable") {
            out.push(p);
        }
    }
}

fn outcome_fingerprint(name: &str, text: &str) -> String {
    match fable::run_capture(name, text) {
        fable::RunOutcome::Ok { stdout, .. } => format!("ok:{stdout}"),
        fable::RunOutcome::Panic { stdout, error } => {
            format!("panic:{}:{stdout}", error.msg)
        }
        fable::RunOutcome::CompileError(diags) => {
            let codes: Vec<&str> =
                diags.iter().filter(|d| d.is_error()).map(|d| d.code).collect();
            format!("err:{}", codes.join(","))
        }
    }
}

#[test]
fn formatter_is_idempotent_and_behavior_preserving() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect(&root.join("examples"), &mut files);
    collect(&root.join("tests/spec"), &mut files);
    assert!(!files.is_empty());

    let mut failures = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let name = f.display().to_string();

        // Skip files that (intentionally) fail to parse — fmt refuses them.
        let Ok(once) = fable::fmt::format_source(&name, &text) else { continue };
        match fable::fmt::format_source(&name, &once) {
            Ok(twice) => {
                if once != twice {
                    failures.push(format!("{name}: not idempotent"));
                    continue;
                }
            }
            Err(_) => {
                failures.push(format!("{name}: formatted output fails to parse"));
                continue;
            }
        }

        // Behavior preservation. Examples that read stdin or are slow in
        // debug builds are exempted from execution (still checked above).
        let base = f.file_name().unwrap().to_string_lossy().to_string();
        if matches!(base.as_str(), "adventure.fable" | "raytracer.fable" | "bench.fable") {
            continue;
        }
        let before = outcome_fingerprint(&name, &text);
        let after = outcome_fingerprint(&name, &once);
        if before != after {
            failures.push(format!(
                "{name}: behavior changed after formatting\n--- before ---\n{before}\n--- after ---\n{after}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} formatter failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Width-aware golden tests
// ---------------------------------------------------------------------------

#[track_caller]
fn fmt_at(width: usize, src: &str) -> String {
    fable::fmt::format_source_width("golden.fable", src, width).expect("golden input must parse")
}

/// Format `src` at `width`, assert the exact golden output, idempotency, and
/// that every line fits in `width` columns.
#[track_caller]
fn golden_at(width: usize, src: &str, want: &str) {
    let once = fmt_at(width, src);
    assert_eq!(once, want, "formatted output differs from the golden string");
    let twice = fmt_at(width, &once);
    assert_eq!(twice, once, "formatting is not idempotent");
    for line in once.lines() {
        assert!(
            line.chars().count() <= width,
            "line exceeds {width} columns: {line:?}"
        );
    }
}

#[track_caller]
fn golden(src: &str, want: &str) {
    golden_at(100, src, want);
}

#[test]
fn width_fifteen_arg_call_breaks_one_per_line() {
    golden(
        "let r = combine(alpha1, alpha2, alpha3, alpha4, alpha5, alpha6, alpha7, alpha8, \
         alpha9, alpha10, alpha11, alpha12, alpha13, alpha14, alpha15);\n",
        "let r = combine(
    alpha1,
    alpha2,
    alpha3,
    alpha4,
    alpha5,
    alpha6,
    alpha7,
    alpha8,
    alpha9,
    alpha10,
    alpha11,
    alpha12,
    alpha13,
    alpha14,
    alpha15,
);
",
    );
}

#[test]
fn width_six_deep_method_chain_breaks_before_each_method_after_the_first() {
    golden(
        "let styled = canvas.rotate(45.0).scale(2.0, 2.0).translate(10.0, 20.0)\
         .recolor(\"teal\").outline(\"black\").finish();\n",
        "let styled = canvas.rotate(45.0)
    .scale(2.0, 2.0)
    .translate(10.0, 20.0)
    .recolor(\"teal\")
    .outline(\"black\")
    .finish();
",
    );
}

#[test]
fn width_nested_literals_break_outermost_first() {
    // The outer map and the wide "server" entry break; the short "client"
    // entry and the inner lists still fit flat on their own lines.
    golden(
        "let config = {\"server\": {\"host\": \"example.internal\", \
         \"ports\": [8080, 8081, 8082, 9090, 9091], \
         \"labels\": [\"alpha\", \"beta\", \"gamma\", \"delta\"]}, \
         \"client\": {\"retries\": 5, \"timeout_ms\": 2500}};\n",
        "let config = {
    \"server\": {
        \"host\": \"example.internal\",
        \"ports\": [8080, 8081, 8082, 9090, 9091],
        \"labels\": [\"alpha\", \"beta\", \"gamma\", \"delta\"],
    },
    \"client\": {\"retries\": 5, \"timeout_ms\": 2500},
};
",
    );
}

#[test]
fn width_long_binary_expression_breaks_before_operators() {
    golden(
        "let total = alpha_component + beta_component + gamma_component + delta_component + \
         epsilon_component;\n",
        "let total = alpha_component
    + beta_component
    + gamma_component
    + delta_component
    + epsilon_component;
",
    );
    // Mixed precedence: only the outermost (lowest-precedence) level breaks.
    golden(
        "let blended = first_weight * first_signal + second_weight * second_signal + \
         third_weight * third_signal;\n",
        "let blended = first_weight * first_signal
    + second_weight * second_signal
    + third_weight * third_signal;
",
    );
    // The binary path is generic over the operator: bitwise (v0.7) breaks
    // the same way, and needed parens survive.
    golden(
        "let packed = (header_word ^ rotation_seed) << 24 | payload_first << 16 | \
         payload_second << 8 | checksum_low_byte;\n",
        "let packed = (header_word ^ rotation_seed) << 24
    | payload_first << 16
    | payload_second << 8
    | checksum_low_byte;
",
    );
}

#[test]
fn width_long_match_arm_and_scrutinee_break_in_place() {
    golden(
        "match request_command {
    Command.Deploy(target, version) -> orchestrate_deployment(target, version, \
         default_timeout_ms, retry_policy, notifier),
    Command.Halt -> shutdown(),
}
",
        "match request_command {
    Command.Deploy(target, version) -> orchestrate_deployment(
        target,
        version,
        default_timeout_ms,
        retry_policy,
        notifier,
    ),
    Command.Halt -> shutdown(),
}
",
    );
    golden(
        "match evaluate_condition(alpha_input, beta_input, gamma_input, delta_input, \
         epsilon_input, zeta_input) {
    true -> \"yes\",
    false -> \"no\",
}
",
        "match evaluate_condition(
    alpha_input,
    beta_input,
    gamma_input,
    delta_input,
    epsilon_input,
    zeta_input,
) {
    true -> \"yes\",
    false -> \"no\",
}
",
    );
}

#[test]
fn width_comments_survive_adjacent_breaks() {
    // A leading comment stays put, a trailing comment (and `//?` directives)
    // re-attach to the last line of the broken statement, and a comment
    // trapped inside a broken literal moves after it — all byte-identical in
    // content and stable under re-formatting.
    golden(
        "// Leading comment stays put.
let result = assemble(first_component, second_component, third_component, fourth_component, \
         fifth_component); // trailing note
println(result); //? expect: assembled
",
        "// Leading comment stays put.
let result = assemble(
    first_component,
    second_component,
    third_component,
    fourth_component,
    fifth_component,
); // trailing note
println(result); //? expect: assembled
",
    );
    golden(
        "let xs = [first_long_element_name, second_long_element_name, third_long_element_name, // note
    fourth_long_element_name];\n",
        "let xs = [
    first_long_element_name,
    second_long_element_name,
    third_long_element_name,
    fourth_long_element_name,
];
// note
",
    );
}

#[test]
fn width_lambda_body_breaks_to_a_block() {
    // The body itself is still too wide for one line, so it also breaks
    // before its operators: groups compose.
    golden(
        "let f = |sample| sample.first_reading * calibration_alpha + \
         sample.second_reading * calibration_beta + drift_offset;\n",
        "let f = |sample| {
    sample.first_reading * calibration_alpha
        + sample.second_reading * calibration_beta
        + drift_offset
};
",
    );
}

#[test]
fn width_hugged_last_arguments() {
    // A block-bodied lambda as the only argument keeps the hugged layout.
    golden(
        "let processed = records.filter(|record| { record.is_active });\n",
        "let processed = records.filter(|record| {
    record.is_active
});
",
    );
    // A too-wide container literal as the last argument hugs the parens.
    golden(
        "let board = make_board([\"row one content here\", \"row two content here\", \
         \"row three content here\", \"row four content here\"]);\n",
        "let board = make_board([
    \"row one content here\",
    \"row two content here\",
    \"row three content here\",
    \"row four content here\",
]);
",
    );
    // Chains with hard multi-line elements stay attached (`}).map(` ...).
    golden(
        "let summary = measurements.filter(|m| { m.valid }).map(|m| { m.value }).join(\", \");\n",
        "let summary = measurements.filter(|m| {
    m.valid
}).map(|m| {
    m.value
}).join(\", \");
",
    );
}

#[test]
fn width_fn_header_params_break_one_per_line() {
    golden(
        "fn interpolate(first_sample: Float, second_sample: Float, blend_factor: Float, \
         easing_curve: String, clamp_output: Bool) -> Float {
    first_sample
}
",
        "fn interpolate(
    first_sample: Float,
    second_sample: Float,
    blend_factor: Float,
    easing_curve: String,
    clamp_output: Bool,
) -> Float {
    first_sample
}
",
    );
}

#[test]
fn width_short_code_keeps_one_line_layout() {
    let src = "let x = add(1, 2);
let names = [\"ada\", \"grace\"];
if ready { launch() } else { wait() }
";
    golden(src, src);
}

#[test]
fn width_parameter_is_respected() {
    // `let value = combine(alpha, beta, gamma);` is exactly 40 columns: it
    // fits at width 40 and breaks at width 39.
    let src = "let value = combine(alpha, beta, gamma);\n";
    golden_at(40, src, src);
    golden_at(
        39,
        src,
        "let value = combine(
    alpha,
    beta,
    gamma,
);
",
    );
}

#[test]
fn width_single_long_token_may_overflow() {
    // A single unbreakable token past the limit is left alone: breaking the
    // call around it would not help (and tokens are never split).
    let src = "report(\"this string literal is one single unbreakable token that stretches \
               far past the hundred column limit on its own\");\n";
    let once = fmt_at(100, src);
    assert_eq!(once, src, "pointless breaks were added around a long token");
    assert_eq!(fmt_at(100, &once), once, "formatting is not idempotent");
}
