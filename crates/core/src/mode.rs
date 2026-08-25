//! Major modes: language + per-mode editing behavior (highlighting,
//! indentation).

/// Languages with a tree-sitter grammar available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Lua,
}

#[derive(Debug, Clone, Copy)]
pub struct Mode {
    pub name: &'static str,
    pub lang: Option<Lang>,
    /// Indent width in spaces; None means no auto-indentation.
    pub indent_unit: Option<usize>,
    /// Line comment prefix (not used yet; reserved).
    pub comment_prefix: Option<&'static str>,
}

pub const FUNDAMENTAL: Mode = Mode {
    name: "Fundamental",
    lang: None,
    indent_unit: None,
    comment_prefix: None,
};

pub const RUST: Mode = Mode {
    name: "Rust",
    lang: Some(Lang::Rust),
    indent_unit: Some(4),
    comment_prefix: Some("//"),
};

pub const LUA: Mode = Mode {
    name: "Lua",
    lang: Some(Lang::Lua),
    indent_unit: Some(4),
    comment_prefix: Some("--"),
};

/// Pick a mode from a file path (or buffer name).
pub fn mode_for_path(path: &str) -> Mode {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".rs") {
        RUST
    } else if lower.ends_with(".lua") {
        LUA
    } else {
        FUNDAMENTAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_extension() {
        assert_eq!(mode_for_path("main.rs").name, "Rust");
        assert_eq!(mode_for_path("/x/y/init.LUA").name, "Lua");
        assert_eq!(mode_for_path("notes.txt").name, "Fundamental");
    }
}
