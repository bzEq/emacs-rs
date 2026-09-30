//! Major modes: language for tree-sitter highlighting plus a local keymap.
//! Mode definitions are registered in the editor's registry from Lua
//! (`define_major_mode`); everything else about a mode (indentation width,
//! comments, ...) lives in Lua.

use crate::keymap::Keymap;

/// Languages with a tree-sitter grammar available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Lua,
    Cpp,
}

/// An active major mode attached to a buffer.
#[derive(Debug, Clone)]
pub struct Mode {
    pub name: String,
    pub lang: Option<Lang>,
}

/// A registered major mode definition (Lua-defined).
#[derive(Debug, Clone, Default)]
pub struct ModeDef {
    pub name: String,
    pub lang: Option<Lang>,
    /// Local keymap installed on buffers using this mode.
    pub keymap: Option<Keymap>,
}

impl ModeDef {
    pub fn to_mode(&self) -> Mode {
        Mode {
            name: self.name.clone(),
            lang: self.lang,
        }
    }
}

pub fn fundamental() -> Mode {
    Mode {
        name: "fundamental-mode".into(),
        lang: None,
    }
}
