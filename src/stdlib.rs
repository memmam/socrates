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
        "std.wav" => include_str!("../std/wav.soc"),
        "std.svg" => include_str!("../std/svg.soc"),
        "std.markdown" => include_str!("../std/markdown.soc"),
        "std.crc" => include_str!("../std/crc.soc"),
        "std.zlib" => include_str!("../std/zlib.soc"),
        "std.png" => include_str!("../std/png.soc"),
        _ => return None,
    })
}

/// Every embedded module key, for error messages and docs.
pub fn std_module_names() -> Vec<&'static str> {
    vec![
        "std.crc",
        "std.deque",
        "std.fft",
        "std.flags",
        "std.glm",
        "std.iter",
        "std.json",
        "std.lazy",
        "std.lists",
        "std.markdown",
        "std.path",
        "std.png",
        "std.set",
        "std.strings",
        "std.svg",
        "std.wav",
        "std.zlib",
    ]
}
