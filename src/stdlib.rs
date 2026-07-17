//! The embedded standard library: Fable modules compiled into the binary and
//! resolved by the loader for `import std.*;` (the `std.` prefix is reserved
//! — it never touches the filesystem or `FABLE_PATH`).

/// The source of an embedded module, by key ("std.json").
pub fn std_module(key: &str) -> Option<&'static str> {
    Some(match key {
        "std.json" => include_str!("../std/json.fable"),
        "std.flags" => include_str!("../std/flags.fable"),
        "std.iter" => include_str!("../std/iter.fable"),
        "std.path" => include_str!("../std/path.fable"),
        "std.strings" => include_str!("../std/strings.fable"),
        "std.lists" => include_str!("../std/lists.fable"),
        "std.set" => include_str!("../std/set.fable"),
        "std.deque" => include_str!("../std/deque.fable"),
        "std.lazy" => include_str!("../std/lazy.fable"),
        "std.glm" => include_str!("../std/glm.fable"),
        "std.fft" => include_str!("../std/fft.fable"),
        _ => return None,
    })
}

/// Every embedded module key, for error messages and docs.
pub fn std_module_names() -> Vec<&'static str> {
    vec![
        "std.deque",
        "std.flags",
        "std.glm",
        "std.iter",
        "std.json",
        "std.lazy",
        "std.lists",
        "std.path",
        "std.set",
        "std.strings",
    ]
}
