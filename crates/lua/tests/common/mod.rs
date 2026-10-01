//! Shared harness for driving the real Lua runtime (the `lua/` defaults
//! plus a script host) against an in-memory Editor, without a terminal.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use emacs_core::editor::Editor;
use emacs_core::key::Key;
use emacs_core::script::{CommandOutcome, PendingRequest, ResumeValue};
use emacs_lua::LuaHost;

pub struct LuaEd {
    pub ed: Editor,
}

/// The repo `lua/` directory (the defaults runtime source of truth).
pub fn lua_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("lua");
    assert!(dir.is_dir(), "lua runtime dir not found at {dir:?}");
    dir
}

impl LuaEd {
    /// An editor with the full defaults runtime loaded (embedded in the
    /// binary, like the shipped `em`).
    pub fn new() -> Self {
        let mut ed = Editor::new(24, 80);
        let host = LuaHost::new().expect("LuaJIT host");
        ed.attach_script(Box::new(host));
        ed.load_runtime().expect("defaults runtime loads");
        LuaEd { ed }
    }

    /// An editor with no Lua runtime loaded (only the raw primitives).
    pub fn bare() -> Self {
        let mut ed = Editor::new(24, 80);
        let host = LuaHost::new().expect("LuaJIT host");
        ed.attach_script(Box::new(host));
        LuaEd { ed }
    }

    /// Load one extra script file into the runtime.
    pub fn load(&mut self, path: &Path) {
        self.ed.load_script(path).expect("script loads");
    }

    /// Run a command; panics on a Lua error.
    pub fn command(&mut self, name: &str) -> CommandOutcome {
        self.command_extra(name, None)
    }

    /// Run a command with a self-insert character.
    pub fn command_extra(&mut self, name: &str, extra: Option<char>) -> CommandOutcome {
        match self.ed.call_command(name, extra) {
            Ok(outcome) => outcome,
            Err(e) => panic!("command {name} failed: {e}"),
        }
    }

    /// Run a command and apply the outcome to the editor.
    pub fn run(&mut self, name: &str) {
        let outcome = self.command(name);
        assert!(
            self.ed.finish_command(outcome).is_none(),
            "unexpected replay"
        );
    }

    /// Run a command expecting it to suspend on a read-string request.
    pub fn command_reading(&mut self, name: &str) {
        let outcome = self.command(name);
        assert!(self.ed.finish_command(outcome).is_none());
        assert!(
            matches!(self.ed.pending(), Some(PendingRequest::ReadString { .. })),
            "expected a read-string request"
        );
    }

    /// Run a command expecting it to suspend on a yes/no request.
    pub fn command_confirming(&mut self, name: &str) {
        let outcome = self.command(name);
        assert!(self.ed.finish_command(outcome).is_none());
        assert!(
            matches!(self.ed.pending(), Some(PendingRequest::ReadYesNo { .. })),
            "expected a yes/no request"
        );
    }

    /// Answer the pending read-string request; returns true if another
    /// read-string request is now pending.
    pub fn answer(&mut self, input: &str) -> bool {
        assert!(
            matches!(self.ed.pending(), Some(PendingRequest::ReadString { .. })),
            "no read-string request pending"
        );
        self.resume(ResumeValue::String(Some(input.to_string())));
        matches!(self.ed.pending(), Some(PendingRequest::ReadString { .. }))
    }

    /// Answer the pending yes/no request; returns true if another yes/no
    /// request is now pending.
    pub fn answer_yes(&mut self, yes: bool) -> bool {
        assert!(
            matches!(self.ed.pending(), Some(PendingRequest::ReadYesNo { .. })),
            "no yes/no request pending"
        );
        self.resume(ResumeValue::Bool(Some(yes)));
        matches!(self.ed.pending(), Some(PendingRequest::ReadYesNo { .. }))
    }

    /// Feed a key to a pending read-key request (isearch, describe-key).
    /// Returns true if the coroutine is still waiting for keys.
    pub fn read_key(&mut self, key: Key) -> bool {
        assert!(
            matches!(self.ed.pending(), Some(PendingRequest::ReadKey)),
            "no read-key request pending"
        );
        self.ed.set_read_key(key);
        self.resume(ResumeValue::Key(key));
        matches!(self.ed.pending(), Some(PendingRequest::ReadKey))
    }

    /// Resume the pending request, apply the outcome, and restore any outer
    /// read the resumed command interrupted (mirrors the app's `resume`).
    pub fn resume(&mut self, value: ResumeValue) {
        let res = self.ed.resume_pending(value).expect("resume");
        assert!(self.ed.finish_command(res.outcome).is_none());
        if let Some(restore) = res.restore {
            self.ed
                .restore_pending_read(restore.request, restore.minibuffer);
        }
    }

    /// Abort all pending input (C-g semantics).
    pub fn abort(&mut self) {
        while self.ed.pending().is_some() {
            self.resume(ResumeValue::String(None));
        }
    }

    pub fn text(&self) -> String {
        self.ed.buf().rope().to_string()
    }

    pub fn point(&self) -> usize {
        self.ed.buf().point()
    }

    /// Run the startup function with a file/directory argument.
    pub fn startup(&mut self, path: Option<&str>) {
        self.ed.run_startup(path).expect("startup");
    }
}
