//! The embedded standard library: Socrates modules compiled into the binary and
//! resolved by the loader for `import std.*;` (the `std.` prefix is reserved
//! — it never touches the filesystem or `SOCRATES_PATH`).

/// The source of an embedded module, by key ("std.json").
pub fn std_module(key: &str) -> Option<&'static str> {
    Some(match key {
        "std.json" => include_str!("../std/json.soc"),
        "std.flags" => include_str!("../std/flags.soc"),
        "std.iter" => include_str!("../std/iter.soc"),
        "std.path" => include_str!("../std/path.soc"),
        "std.strings" => include_str!("../std/strings.soc"),
        "std.lists" => include_str!("../std/lists.soc"),
        "std.set" => include_str!("../std/set.soc"),
        "std.deque" => include_str!("../std/deque.soc"),
        "std.lazy" => include_str!("../std/lazy.soc"),
        "std.glm" => include_str!("../std/glm.soc"),
        "std.fft" => include_str!("../std/fft.soc"),
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
