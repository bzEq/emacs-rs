//! The command registry: named commands callable via key bindings or M-x.

use std::collections::HashMap;

use anyhow::Result;

use crate::editor::Editor;

pub type CommandFn = fn(&mut Editor) -> Result<()>;

#[derive(Clone)]
pub struct Command {
    pub name: String,
    /// Rust implementation, if any.
    pub rust_fn: Option<CommandFn>,
    /// Id of the Lua closure implementing this command, if any.
    pub lua_id: Option<u32>,
    /// One-line documentation, shown by describe-key.
    pub doc: &'static str,
}

#[derive(Default)]
pub struct CommandRegistry {
    map: HashMap<String, Command>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, name: impl Into<String>, doc: &'static str, f: CommandFn) {
        let name = name.into();
        self.map.insert(
            name.clone(),
            Command {
                name,
                rust_fn: Some(f),
                lua_id: None,
                doc,
            },
        );
    }

    pub fn add_lua(&mut self, name: impl Into<String>, id: u32) {
        let name = name.into();
        self.map.insert(
            name.clone(),
            Command {
                name,
                rust_fn: None,
                lua_id: Some(id),
                doc: "",
            },
        );
    }

    pub fn get(&self, name: &str) -> Option<&Command> {
        self.map.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }

    /// Names of all commands whose name starts with `prefix` (or contains it
    /// if `prefix` has no exact prefix matches), for minibuffer completion.
    pub fn complete(&self, prefix: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .map
            .keys()
            .filter(|n| n.starts_with(prefix))
            .cloned()
            .collect();
        if names.is_empty() {
            names = self
                .map
                .keys()
                .filter(|n| n.contains(prefix))
                .cloned()
                .collect();
        }
        names.sort();
        names
    }
}
