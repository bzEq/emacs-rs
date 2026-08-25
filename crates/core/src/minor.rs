//! Minor modes: per-buffer toggleable behaviors with optional keymaps.
//! Definitions are registered in the editor's registry so Lua can define
//! new ones; `line-numbers` is built in.

use crate::keymap::Keymap;

#[derive(Debug, Clone, Default)]
pub struct MinorModeDef {
    pub name: String,
    pub doc: String,
    /// Short string shown in the modeline while enabled.
    pub lighter: String,
    /// Keymap active while the mode is enabled.
    pub keymap: Option<Keymap>,
}

pub fn line_numbers_def() -> MinorModeDef {
    MinorModeDef {
        name: "line-numbers".into(),
        doc: "Display line numbers in the gutter.".into(),
        lighter: "Ln".into(),
        keymap: None,
    }
}
