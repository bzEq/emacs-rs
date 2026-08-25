//! Major modes: language + per-mode editing behavior (highlighting,
//! indentation, local keymap). Mode definitions are registered in the
//! editor's registry so Lua can define new ones.

use crate::keymap::Keymap;

/// Languages with a tree-sitter grammar available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Lua,
}

/// An active major mode attached to a buffer.
#[derive(Debug, Clone)]
pub struct Mode {
    pub name: String,
    pub lang: Option<Lang>,
    /// Indent width in spaces; None means no auto-indentation.
    pub indent_unit: Option<usize>,
    /// Line comment prefix (not used yet; reserved).
    pub comment_prefix: Option<String>,
}

/// A registered major mode definition (built-in or Lua-defined).
#[derive(Debug, Clone, Default)]
pub struct ModeDef {
    pub name: String,
    pub lang: Option<Lang>,
    pub indent_unit: Option<usize>,
    pub comment_prefix: Option<String>,
    /// Local keymap installed on buffers using this mode.
    pub keymap: Option<Keymap>,
}

impl ModeDef {
    pub fn to_mode(&self) -> Mode {
        Mode {
            name: self.name.clone(),
            lang: self.lang,
            indent_unit: self.indent_unit,
            comment_prefix: self.comment_prefix.clone(),
        }
    }
}

pub fn fundamental_def() -> ModeDef {
    ModeDef {
        name: "fundamental-mode".into(),
        lang: None,
        indent_unit: None,
        comment_prefix: None,
        keymap: None,
    }
}

pub fn rust_def() -> ModeDef {
    ModeDef {
        name: "rust-mode".into(),
        lang: Some(Lang::Rust),
        indent_unit: Some(4),
        comment_prefix: Some("//".into()),
        keymap: None,
    }
}

pub fn lua_def() -> ModeDef {
    ModeDef {
        name: "lua-mode".into(),
        lang: Some(Lang::Lua),
        indent_unit: Some(4),
        comment_prefix: Some("--".into()),
        keymap: None,
    }
}

pub fn fundamental() -> Mode {
    fundamental_def().to_mode()
}

pub fn rust() -> Mode {
    rust_def().to_mode()
}

pub fn lua() -> Mode {
    lua_def().to_mode()
}

/// Pick a mode from a file path (or buffer name).
pub fn mode_for_path(path: &str) -> Mode {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".rs") {
        rust()
    } else if lower.ends_with(".lua") {
        lua()
    } else {
        fundamental()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_extension() {
        assert_eq!(mode_for_path("main.rs").name, "rust-mode");
        assert_eq!(mode_for_path("/x/y/init.LUA").name, "lua-mode");
        assert_eq!(mode_for_path("notes.txt").name, "fundamental-mode");
    }
}
