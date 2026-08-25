//! The Editor: all global editor state — buffers, keymap, commands, kill
//! ring, echo area, minibuffer, prefix argument, views, and the optional
//! scripting host.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::buffer::Buffer;
use crate::command::CommandRegistry;
use crate::key::Key;
use crate::keymap::Keymap;
use crate::kill_ring::KillRing;
use crate::minibuffer::{BoolContinuation, CompletionFn, Minibuffer, Pending, StringContinuation};
use crate::script::{NullHost, ScriptHost};
use crate::view::View;

/// Numeric prefix argument state (C-u, C-3, M--, ...).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrefixArg {
    pub digits: Option<u64>,
    pub negative: bool,
    pub universal: u32,
}

impl PrefixArg {
    pub fn is_active(&self) -> bool {
        self.digits.is_some() || self.negative || self.universal > 0
    }

    /// The final numeric value, Emacs-style: digits win; `C-u` is 4^n; bare
    /// `C--` is -1; no argument is 1.
    pub fn value(&self) -> i64 {
        if let Some(d) = self.digits {
            return if self.negative { -(d as i64) } else { d as i64 };
        }
        if self.negative {
            return -1;
        }
        if self.universal > 0 {
            return 4i64.pow(self.universal);
        }
        1
    }
}

pub struct Editor {
    buffers: Vec<Buffer>,
    current: usize,
    keymap: Keymap,
    commands: CommandRegistry,
    kill_ring: KillRing,
    /// (pos, len) of the last yank, for yank-pop.
    last_yank: Option<(usize, usize)>,
    /// Message in the echo area, if any.
    echo: Option<String>,
    /// True if `echo` is an error.
    echo_error: bool,
    minibuffer: Option<Minibuffer>,
    pending: Option<Pending>,
    /// Keys of the key sequence in progress (for prefix resolution).
    pending_keys: Vec<Key>,
    /// Esc acts as a Meta prefix (ESC x == M-x).
    esc_prefix: bool,
    prefix_arg: PrefixArg,
    this_command: String,
    last_command: String,
    /// Char passed to self-insert-command.
    self_insert_char: Option<char>,
    quit: bool,
    /// Per-buffer window scroll state, keyed by buffer id.
    views: HashMap<usize, View>,
    /// Rows/cols of the buffer area (total terminal size minus echo line).
    window_rows: usize,
    window_cols: usize,
    script: Option<Box<dyn ScriptHost>>,
}

impl Editor {
    pub fn new(window_rows: usize, window_cols: usize) -> Self {
        let mut ed = Editor {
            buffers: vec![Buffer::new("*scratch*")],
            current: 0,
            keymap: Keymap::new(),
            commands: CommandRegistry::new(),
            kill_ring: KillRing::new(),
            last_yank: None,
            echo: None,
            echo_error: false,
            minibuffer: None,
            pending: None,
            pending_keys: Vec::new(),
            esc_prefix: false,
            prefix_arg: PrefixArg::default(),
            this_command: String::new(),
            last_command: String::new(),
            self_insert_char: None,
            quit: false,
            views: HashMap::new(),
            window_rows,
            window_cols,
            script: None,
        };
        crate::commands::register_defaults(&mut ed);
        ed
    }

    // --- buffers -----------------------------------------------------------

    pub fn buffers(&self) -> &[Buffer] {
        &self.buffers
    }

    pub fn buffers_mut(&mut self) -> &mut [Buffer] {
        &mut self.buffers
    }

    pub fn buf(&self) -> &Buffer {
        &self.buffers[self.current]
    }

    pub fn buf_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }

    pub fn set_current(&mut self, idx: usize) {
        self.current = idx.min(self.buffers.len() - 1);
    }

    pub fn current_idx(&self) -> usize {
        self.current
    }

    /// Switch to a buffer by name; creates it if `create` is true.
    pub fn switch_to_buffer(&mut self, name: &str, create: bool) -> Result<()> {
        if let Some(idx) = self.buffers.iter().position(|b| b.name() == name) {
            self.current = idx;
            return Ok(());
        }
        if !create {
            return Err(anyhow::anyhow!("no buffer named {name}"));
        }
        self.buffers.push(Buffer::new(name.to_string()));
        self.current = self.buffers.len() - 1;
        Ok(())
    }

    /// Find a buffer by file path.
    pub fn find_buffer_by_path(&self, path: &Path) -> Option<usize> {
        self.buffers
            .iter()
            .position(|b| b.path() == Some(path))
    }

    pub fn add_buffer(&mut self, buf: Buffer) -> usize {
        self.buffers.push(buf);
        self.buffers.len() - 1
    }

    /// Remove the buffer at `idx` (caller handles switching).
    pub fn remove_buffer(&mut self, idx: usize) {
        self.buffers.remove(idx);
    }

    pub fn remove_view(&mut self, buffer_id: usize) {
        self.views.remove(&buffer_id);
    }

    // --- views -------------------------------------------------------------

    pub fn view(&self) -> &View {
        self.views
            .get(&self.buf().id)
            .unwrap_or(&View::DEFAULT)
    }

    pub fn view_mut(&mut self) -> &mut View {
        self.views.entry(self.buf().id).or_default()
    }

    pub fn window_rows(&self) -> usize {
        self.window_rows
    }

    pub fn window_cols(&self) -> usize {
        self.window_cols
    }

    pub fn set_window_size(&mut self, rows: usize, cols: usize) {
        self.window_rows = rows;
        self.window_cols = cols;
    }

    /// Keep the current buffer's view scrolled so the cursor is visible.
    pub fn scroll_current_view(&mut self) {
        let rows = self.window_rows;
        let buf = &mut self.buffers[self.current];
        let view = self.views.entry(buf.id).or_default();
        view.scroll_to_cursor(buf, rows);
    }

    pub fn page_down_current(&mut self) {
        let rows = self.window_rows;
        let buf = &mut self.buffers[self.current];
        let view = self.views.entry(buf.id).or_default();
        view.page_down(buf, rows);
    }

    pub fn page_up_current(&mut self) {
        let rows = self.window_rows;
        let buf = &mut self.buffers[self.current];
        let view = self.views.entry(buf.id).or_default();
        view.page_up(buf, rows);
    }

    pub fn recenter_current(&mut self) {
        let rows = self.window_rows;
        let buf = &self.buffers[self.current];
        let view = self.views.entry(buf.id).or_default();
        view.recenter(buf, rows);
    }

    // --- keymap / commands -------------------------------------------------

    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    pub fn keymap_mut(&mut self) -> &mut Keymap {
        &mut self.keymap
    }

    pub fn commands(&self) -> &CommandRegistry {
        &self.commands
    }

    pub fn commands_mut(&mut self) -> &mut CommandRegistry {
        &mut self.commands
    }

    /// Invoke a command by name: undo bookkeeping, last-command tracking,
    /// prefix-arg consumption, then dispatch to Rust or the script host.
    pub fn invoke_command(&mut self, name: &str) -> Result<()> {
        let cmd = match self.commands.get(name) {
            Some(c) => c.clone(),
            None => return Err(anyhow::anyhow!("{name} is undefined")),
        };
        if self.last_command != name {
            self.buffers[self.current].undo_boundary();
        }
        self.last_command = std::mem::take(&mut self.this_command);
        self.this_command = name.to_string();
        let result = match cmd.lua_id {
            Some(id) => self.with_host(|ed, host| host.call_command(id, ed)),
            None => cmd.rust_fn.expect("command has no implementation")(self),
        };
        // last-command is the command that just ran (Emacs updates it after
        // execution; during execution it still refers to the previous one).
        self.last_command = name.to_string();
        // The prefix arg applies to the next command only; the commands that
        // set it keep it for that next command.
        let is_prefix_setter = name == "universal-argument"
            || name == "negative-argument"
            || name.starts_with("digit-argument-");
        if !is_prefix_setter {
            self.prefix_arg = PrefixArg::default();
        }
        result
    }

    // --- key sequence state ------------------------------------------------

    pub fn pending_keys(&self) -> &[Key] {
        &self.pending_keys
    }

    pub fn push_key(&mut self, key: Key) {
        self.pending_keys.push(key);
    }

    pub fn clear_pending_keys(&mut self) {
        self.pending_keys.clear();
        self.esc_prefix = false;
    }

    pub fn esc_prefix(&self) -> bool {
        self.esc_prefix
    }

    pub fn set_esc_prefix(&mut self, v: bool) {
        self.esc_prefix = v;
    }

    pub fn self_insert_char(&self) -> Option<char> {
        self.self_insert_char
    }

    pub fn set_self_insert_char(&mut self, c: Option<char>) {
        self.self_insert_char = c;
    }

    pub fn prefix_arg(&self) -> PrefixArg {
        self.prefix_arg
    }

    pub fn set_prefix_arg(&mut self, arg: PrefixArg) {
        self.prefix_arg = arg;
    }

    pub fn this_command(&self) -> &str {
        &self.this_command
    }

    pub fn last_command(&self) -> &str {
        &self.last_command
    }

    pub fn quit(&self) -> bool {
        self.quit
    }

    pub fn set_quit(&mut self, q: bool) {
        self.quit = q;
    }

    // --- echo area ---------------------------------------------------------

    pub fn message(&mut self, msg: impl Into<String>) {
        self.echo = Some(msg.into());
        self.echo_error = false;
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        self.echo = Some(msg.into());
        self.echo_error = true;
    }

    pub fn echo(&self) -> Option<&str> {
        self.echo.as_deref()
    }

    pub fn echo_is_error(&self) -> bool {
        self.echo_error
    }

    /// Clear the echo area (called before running the next command).
    pub fn clear_echo(&mut self) {
        self.echo = None;
        self.echo_error = false;
    }

    // --- minibuffer / pending ----------------------------------------------

    pub fn minibuffer(&self) -> Option<&Minibuffer> {
        self.minibuffer.as_ref()
    }

    pub fn minibuffer_mut(&mut self) -> Option<&mut Minibuffer> {
        self.minibuffer.as_mut()
    }

    pub fn pending(&self) -> Option<&Pending> {
        self.pending.as_ref()
    }

    pub fn pending_mut(&mut self) -> Option<&mut Pending> {
        self.pending.as_mut()
    }

    pub fn set_pending(&mut self, pending: Option<Pending>) {
        self.pending = pending;
    }

    pub fn take_pending(&mut self) -> Option<Pending> {
        self.pending.take()
    }

    /// Ask the minibuffer for a string; `cont` runs with the answer.
    pub fn read_string(
        &mut self,
        prompt: impl Into<String>,
        completion: Option<CompletionFn>,
        cont: StringContinuation,
    ) {
        self.minibuffer = Some(Minibuffer::new(prompt.into(), completion));
        self.pending = Some(Pending::ReadString { cont });
    }

    pub fn read_yes_no(&mut self, prompt: impl Into<String>, cont: BoolContinuation) {
        self.pending = Some(Pending::YesNo {
            prompt: prompt.into(),
            cont,
        });
    }

    /// Minibuffer accepted (RET): run the continuation.
    pub fn finish_read_string(&mut self, input: String) -> Result<()> {
        let pending = self.pending.take();
        self.minibuffer = None;
        match pending {
            Some(Pending::ReadString { cont, .. }) => cont(self, input),
            _ => Ok(()),
        }
    }

    /// Abort any minibuffer/pending state (C-g).
    pub fn abort_pending(&mut self) {
        self.pending = None;
        self.minibuffer = None;
        self.clear_pending_keys();
    }

    // --- kill ring ---------------------------------------------------------

    pub fn kill_ring(&self) -> &KillRing {
        &self.kill_ring
    }

    pub fn kill_ring_mut(&mut self) -> &mut KillRing {
        &mut self.kill_ring
    }

    /// Kill `text`; appends to the current entry if the last command was a
    /// kill command (Emacs accumulates consecutive kills).
    pub fn kill(&mut self, text: String) {
        let append = matches!(
            self.last_command(),
            "kill-line" | "kill-region" | "kill-word" | "backward-kill-word"
        );
        self.kill_ring.kill(text, append);
    }

    pub fn last_yank(&self) -> Option<(usize, usize)> {
        self.last_yank
    }

    pub fn set_last_yank(&mut self, y: Option<(usize, usize)>) {
        self.last_yank = y;
    }

    // --- script host -------------------------------------------------------

    pub fn attach_script(&mut self, host: Box<dyn ScriptHost>) {
        self.script = Some(host);
    }

    /// Borrow the script host and the editor at once. The host is taken out
    /// of the editor during the call so the borrows don't conflict.
    pub fn with_host<R>(&mut self, f: impl FnOnce(&mut Editor, &mut dyn ScriptHost) -> R) -> R {
        let mut host = self.script.take();
        let res = match host.as_mut() {
            Some(h) => f(self, h.as_mut()),
            None => f(self, &mut NullHost),
        };
        self.script = host;
        res
    }

    /// Run a hook (e.g. "before_save") if a script host is attached.
    pub fn run_hook(&mut self, name: &str) -> Result<()> {
        let mut res = Ok(());
        let _ = self.with_host(|ed, host| {
            res = host.call_hook(name, ed);
        });
        res
    }

    /// Load a script file (init.lua).
    pub fn load_script(&mut self, path: &Path) -> Result<()> {
        let mut res = Ok(());
        let _ = self.with_host(|ed, host| {
            res = host.load_file(path, ed);
        });
        res
    }

    /// Convenience: mark the current buffer read-only (for *Help*).
    pub fn current_buf_path(&self) -> Option<PathBuf> {
        self.buf().path().map(|p| p.to_path_buf())
    }
}

impl View {
    pub const DEFAULT: View = View { top_line: 0 };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_arg_values() {
        assert_eq!(PrefixArg::default().value(), 1);
        assert_eq!(
            PrefixArg { universal: 1, ..Default::default() }.value(),
            4
        );
        assert_eq!(
            PrefixArg { universal: 2, ..Default::default() }.value(),
            16
        );
        assert_eq!(
            PrefixArg { digits: Some(12), ..Default::default() }.value(),
            12
        );
        assert_eq!(
            PrefixArg { digits: Some(3), negative: true, ..Default::default() }.value(),
            -3
        );
        assert_eq!(PrefixArg { negative: true, ..Default::default() }.value(), -1);
    }

    #[test]
    fn buffer_switch_create() {
        let mut ed = Editor::new(20, 80);
        assert_eq!(ed.buf().name(), "*scratch*");
        ed.switch_to_buffer("foo.txt", true).unwrap();
        assert_eq!(ed.buf().name(), "foo.txt");
        ed.switch_to_buffer("*scratch*", false).unwrap();
        assert_eq!(ed.buf().name(), "*scratch*");
    }

    #[test]
    fn invoke_tracks_last_command() {
        let mut ed = Editor::new(20, 80);
        ed.invoke_command("forward-char").unwrap();
        assert_eq!(ed.last_command(), "forward-char");
        ed.invoke_command("nosuchcommand").unwrap_err();
    }

    #[test]
    fn esc_prefix_key() {
        let mut ed = Editor::new(20, 80);
        ed.set_esc_prefix(true);
        assert!(ed.esc_prefix());
        ed.set_esc_prefix(false);
        assert!(!ed.esc_prefix());
    }
}
