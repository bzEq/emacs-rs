//! Pluggable scripting engine (LuaJIT). The editor's command loop calls
//! into the host through this trait; the host drives commands as Lua
//! coroutines and calls back into `Editor` through the `&mut Editor`
//! argument. This keeps `emacs-core` free of any scripting dependency.

use std::path::Path;

use anyhow::Result;

use crate::editor::Editor;
use crate::key::Key;

/// What a suspended command coroutine is waiting for. The event loop routes
/// keys to the matching handler until the coroutine is resumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingRequest {
    /// Reading a string from the minibuffer; completion may be active and
    /// the input may be pre-filled with `initial` (cursor at its end).
    ReadString {
        prompt: String,
        completion: bool,
        initial: String,
    },
    /// A yes/no question.
    ReadYesNo { prompt: String },
    /// A raw key press (isearch, describe-key).
    ReadKey,
}

/// The value handed back to a suspended coroutine.
#[derive(Debug, Clone)]
pub enum ResumeValue {
    String(Option<String>),
    Bool(Option<bool>),
    Key(Key),
}

/// The result of running (part of) a command coroutine.
#[derive(Debug)]
pub enum CommandOutcome {
    /// The command finished.
    Done,
    /// The command suspended, waiting for input.
    Pending(PendingRequest),
}

pub trait ScriptHost {
    /// Load a script file (a Lua chunk).
    fn load_file(&mut self, path: &Path, editor: &mut Editor) -> Result<()>;

    /// Load the required runtime modules embedded in the binary, in the
    /// order the engine defines. A broken module is a fatal error.
    fn load_runtime(&mut self, editor: &mut Editor) -> Result<()>;

    /// Load the required runtime modules from a directory instead (the
    /// `EMACS_RS_LUA_DIR` override). A missing or broken module is a
    /// fatal error.
    fn load_runtime_dir(&mut self, dir: &Path, editor: &mut Editor) -> Result<()>;

    /// Invoke a command by name. `extra` carries the character for
    /// self-insert-command.
    fn call_command(
        &mut self,
        name: &str,
        extra: Option<char>,
        editor: &mut Editor,
    ) -> Result<CommandOutcome>;

    /// Resume a suspended command coroutine with the requested input.
    fn resume_pending(&mut self, value: ResumeValue, editor: &mut Editor)
        -> Result<CommandOutcome>;

    /// Recompute minibuffer completion candidates for the current input.
    fn update_completion(&mut self, input: &str, editor: &mut Editor) -> Result<Vec<String>>;

    /// Run the Lua `startup` function with the command-line file argument.
    fn run_startup(&mut self, path: Option<&str>, editor: &mut Editor) -> Result<()>;

    /// Notify the scripting host that the files of the given buffers
    /// changed on disk (inotify events); the Lua side applies the
    /// auto-revert policy.
    fn notify_file_changes(&mut self, ids: &[usize], editor: &mut Editor) -> Result<()>;
}

/// No-op host used when no scripting engine is attached.
pub struct NullHost;

impl ScriptHost for NullHost {
    fn load_file(&mut self, _path: &Path, _editor: &mut Editor) -> Result<()> {
        Ok(())
    }

    fn load_runtime(&mut self, _editor: &mut Editor) -> Result<()> {
        Ok(())
    }

    fn load_runtime_dir(&mut self, _dir: &Path, _editor: &mut Editor) -> Result<()> {
        Ok(())
    }

    fn call_command(
        &mut self,
        _name: &str,
        _extra: Option<char>,
        _editor: &mut Editor,
    ) -> Result<CommandOutcome> {
        Ok(CommandOutcome::Done)
    }

    fn resume_pending(
        &mut self,
        _value: ResumeValue,
        _editor: &mut Editor,
    ) -> Result<CommandOutcome> {
        Ok(CommandOutcome::Done)
    }

    fn update_completion(&mut self, _input: &str, _editor: &mut Editor) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn run_startup(&mut self, _path: Option<&str>, _editor: &mut Editor) -> Result<()> {
        Ok(())
    }

    fn notify_file_changes(&mut self, _ids: &[usize], _editor: &mut Editor) -> Result<()> {
        Ok(())
    }
}
