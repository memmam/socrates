//! CLI: `socrates test --bless` (v0.8) — rewrites mismatched `//? expect:`
//! lines in place instead of failing, when the actual/expected line counts
//! already agree. The automatable half of "generate long pin blocks
//! mechanically" (demos/STYLE.md § 1).

#[test]
fn bless_rewrites_matching_line_count() {
    let dir = std::env::temp_dir().join(format!("socrates-bless-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.soc");
    std::fs::write(
        &f,
        "println(1 + 1);       //? expect: 3\n\
         println(\"hi\");        //? expect: bye\n\
         println(2 * 3);       //? expect: 6\n",
    )
    .unwrap();

    // Without --bless, the mismatch fails and the file is untouched.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_socrates"))
        .args(["test"])
        .arg(&f)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "println(1 + 1);       //? expect: 3\n\
         println(\"hi\");        //? expect: bye\n\
         println(2 * 3);       //? expect: 6\n",
    );

    // --bless rewrites only the two mismatched lines; the already-correct
    // third line is untouched (still exits 0 — a bless run isn't a failure).
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_socrates"))
        .args(["test", "--bless"])
        .arg(&f)
        .output()
        .unwrap();
    assert!(out.status.success(), "bless run failed: {out:?}");
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "println(1 + 1);       //? expect: 2\n\
         println(\"hi\");        //? expect: hi\n\
         println(2 * 3);       //? expect: 6\n",
    );

    // The blessed file now passes on its own.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_socrates"))
        .args(["test"])
        .arg(&f)
        .output()
        .unwrap();
    assert!(out.status.success(), "re-run after bless failed: {out:?}");
}

#[test]
fn bless_refuses_a_line_count_change() {
    let dir = std::env::temp_dir().join(format!("socrates-bless-cli-count-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.soc");
    let original = "println(1);       //? expect: 99\nprintln(2);\nprintln(3);\n";
    std::fs::write(&f, original).unwrap();

    // 1 expect line, 3 actual output lines: bless can't tell which lines the
    // new output corresponds to, so it leaves the file alone and still fails.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_socrates"))
        .args(["test", "--bless"])
        .arg(&f)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(std::fs::read_to_string(&f).unwrap(), original);
}

#[test]
fn bless_leaves_error_and_panic_directives_alone() {
    let dir = std::env::temp_dir().join(format!("socrates-bless-cli-panic-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("t.soc");
    // Wrong panic substring — --bless must not silently "fix" this by
    // rewriting the panic directive; only `expect:` is ever rewritten.
    let original = "//? panic: this is not the message\nprintln(1);\npanic(\"boom\");\n";
    std::fs::write(&f, original).unwrap();

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_socrates"))
        .args(["test", "--bless"])
        .arg(&f)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(std::fs::read_to_string(&f).unwrap(), original);
}

#[test]
fn unknown_flag_still_rejected_alongside_bless() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_socrates"))
        .args(["test", "--bogus"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--bless"));
}
