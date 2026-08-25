//! Trait for pluggable scripting engines (LuaJIT in M1). `Editor` calls
//! into the host through this trait; the host calls back into `Editor`
//! through the `&mut Editor` argument. This keeps `emacs-core` free of any
//! scripting dependency.

use std::path::Path;

use anyhow::Result;

use crate::editor::Editor;

pub trait ScriptHost {
    /// Invoke a script-defined command previously registered with `id`.
    fn call_command(&mut self, id: u32, editor: &mut Editor) -> Result<()>;

    /// Run all functions registered for a hook (e.g. "before_save").
    fn call_hook(&mut self, name: &str, editor: &mut Editor) -> Result<()>;

    /// Load a script file (e.g. `init.lua`).
    fn load_file(&mut self, path: &Path, editor: &mut Editor) -> Result<()>;
}

/// No-op host used when no scripting engine is attached.
pub struct NullHost;

impl ScriptHost for NullHost {
    fn call_command(&mut self, _id: u32, _editor: &mut Editor) -> Result<()> {
        Ok(())
    }

    fn call_hook(&mut self, _name: &str, _editor: &mut Editor) -> Result<()> {
        Ok(())
    }

    fn load_file(&mut self, _path: &Path, _editor: &mut Editor) -> Result<()> {
        Ok(())
    }
}
