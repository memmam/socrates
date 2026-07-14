//! The embedded standard library: Fable modules compiled into the binary and
//! resolved by the loader for `import std.*;` (the `std.` prefix is reserved
//! — it never touches the filesystem or `FABLE_PATH`).

/// The source of an embedded module, by key ("std.json").
pub fn std_module(key: &str) -> Option<&'static str> {
    Some(match key {
        "std.json" => include_str!("../std/json.fable"),
        "std.flags" => include_str!("../std/flags.fable"),
        "std.path" => include_str!("../std/path.fable"),
        "std.strings" => include_str!("../std/strings.fable"),
        _ => return None,
    })
}

/// Every embedded module key, for error messages and docs.
pub fn std_module_names() -> Vec<&'static str> {
    vec!["std.flags", "std.json", "std.path", "std.strings"]
}
