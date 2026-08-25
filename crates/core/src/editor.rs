//! The Editor: all global editor state — buffers, windows, keymap, commands,
//! kill ring, echo area, minibuffer, prefix argument, and the optional
//! scripting host.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::buffer::Buffer;
use crate::command::CommandRegistry;
use crate::isearch::ISearch;
use crate::key::Key;
use crate::keymap::Keymap;
use crate::kill_ring::KillRing;
use crate::minibuffer::{BoolContinuation, CompletionFn, Minibuffer, Pending, StringContinuation};
use crate::script::{NullHost, ScriptHost};
use crate::view::View;
use crate::window::{Rect as WinRect, Split, WindowTree};

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

/// One window's rendering info, handed to the UI.
pub struct WindowLayout<'a> {
    pub buf: &'a Buffer,
    pub view: &'a View,
    pub rect: WinRect,
    pub selected: bool,
}

pub struct Editor {
    buffers: Vec<Buffer>,
    windows: WindowTree,
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
    /// Rows/cols of the buffer area (terminal size minus modeline + echo).
    window_rows: usize,
    window_cols: usize,
    /// Active incremental search state.
    isearch: Option<ISearch>,
    script: Option<Box<dyn ScriptHost>>,
}

impl Editor {
    pub fn new(window_rows: usize, window_cols: usize) -> Self {
        let scratch = Buffer::new("*scratch*");
        let scratch_id = scratch.id;
        let mut ed = Editor {
            buffers: vec![scratch],
            windows: WindowTree::new(scratch_id),
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
            window_rows,
            window_cols,
            isearch: None,
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

    /// Index into `buffers` of the buffer with the given id.
    pub fn buffer_index(&self, id: usize) -> usize {
        self.buffers
            .iter()
            .position(|b| b.id == id)
            .expect("buffer id exists")
    }

    /// The buffer shown in the selected window.
    pub fn buf(&self) -> &Buffer {
        let id = self.windows.selected_buffer();
        &self.buffers[self.buffer_index(id)]
    }

    pub fn buf_mut(&mut self) -> &mut Buffer {
        let id = self.windows.selected_buffer();
        let idx = self.buffer_index(id);
        &mut self.buffers[idx]
    }

    pub fn selected_buffer_id(&self) -> usize {
        self.windows.selected_buffer()
    }

    pub fn selected_buffer_index(&self) -> usize {
        let id = self.windows.selected_buffer();
        self.buffer_index(id)
    }

    /// Show `id` in the selected window, preserving window-points.
    pub fn set_selected_buffer(&mut self, id: usize) {
        let old_id = self.windows.selected_buffer();
        if old_id == id {
            return;
        }
        let old_idx = self.buffer_index(old_id);
        let new_idx = self.buffer_index(id);
        let point = self.buffers[old_idx].point();
        let w = self.windows.selected_mut();
        w.point = Some(point);
        w.buffer = id;
        let saved = w.point.take();
        if let Some(p) = saved {
            self.buffers[new_idx].set_point(p);
        }
    }

    /// Switch the selected window to a buffer by name; creates it if
    /// `create` is true.
    pub fn switch_to_buffer(&mut self, name: &str, create: bool) -> Result<()> {
        if let Some(idx) = self.buffers.iter().position(|b| b.name() == name) {
            let id = self.buffers[idx].id;
            self.set_selected_buffer(id);
            return Ok(());
        }
        if !create {
            return Err(anyhow::anyhow!("no buffer named {name}"));
        }
        let buf = Buffer::new(name.to_string());
        let id = buf.id;
        self.buffers.push(buf);
        self.set_selected_buffer(id);
        Ok(())
    }

    /// Find a buffer by file path.
    pub fn find_buffer_by_path(&self, path: &Path) -> Option<usize> {
        self.buffers.iter().position(|b| b.path() == Some(path))
    }

    pub fn add_buffer(&mut self, buf: Buffer) -> usize {
        self.buffers.push(buf);
        self.buffers.len() - 1
    }

    /// Remove the buffer at `idx` (caller handles windows).
    pub fn remove_buffer(&mut self, idx: usize) {
        self.buffers.remove(idx);
    }

    /// Point all windows showing `old_id` at `new_id` (buffer killed).
    pub fn replace_buffer_in_windows(&mut self, old_id: usize, new_id: usize) {
        self.windows.replace_buffer(old_id, new_id);
    }

    // --- windows -----------------------------------------------------------

    pub fn window_layout(&self) -> Vec<WindowLayout<'_>> {
        let body = self.body_rect();
        let selected = self.windows.selected_path().to_vec();
        self.windows
            .layout(body)
            .into_iter()
            .map(|(path, w, rect)| {
                let idx = self.buffer_index(w.buffer);
                WindowLayout {
                    buf: &self.buffers[idx],
                    view: &w.view,
                    rect,
                    selected: path == selected,
                }
            })
            .collect()
    }

    pub fn body_rect(&self) -> WinRect {
        WinRect {
            x: 0,
            y: 0,
            w: self.window_cols as u16,
            h: self.window_rows as u16,
        }
    }

    /// Height of the selected window, for scrolling commands.
    pub fn selected_window_height(&self) -> usize {
        self.window_layout()
            .into_iter()
            .find(|l| l.selected)
            .map(|l| l.rect.h as usize)
            .unwrap_or(self.window_rows)
            .max(1)
    }

    pub fn split_window(&mut self, split: Split) {
        let point = self.buf().point();
        self.windows.split(split, point);
    }

    pub fn delete_window(&mut self) -> bool {
        self.windows.delete_selected()
    }

    pub fn delete_other_windows(&mut self) {
        self.windows.delete_others();
    }

    /// Cycle to the next window (C-x o), preserving window-points. Returns
    /// false if there is only one window.
    pub fn other_window(&mut self) -> bool {
        let old_id = self.windows.selected_buffer();
        let old_idx = self.buffer_index(old_id);
        let point = self.buffers[old_idx].point();
        self.windows.selected_mut().point = Some(point);
        if !self.windows.next() {
            return false;
        }
        let new_id = self.windows.selected_buffer();
        let new_idx = self.buffer_index(new_id);
        let saved = self.windows.selected().point;
        if let Some(p) = saved {
            self.buffers[new_idx].set_point(p);
            self.windows.selected_mut().point = None;
        }
        true
    }

    pub fn single_window(&self) -> bool {
        self.windows.is_single()
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

    /// Keep the selected window's view scrolled so the cursor is visible.
    pub fn scroll_current_view(&mut self) {
        let id = self.windows.selected_buffer();
        let idx = self.buffer_index(id);
        let rows = self.selected_window_height();
        let buf = &mut self.buffers[idx];
        let w = self.windows.selected_mut();
        w.view.scroll_to_cursor(buf, rows);
    }

    pub fn page_down_current(&mut self) {
        let id = self.windows.selected_buffer();
        let idx = self.buffer_index(id);
        let rows = self.selected_window_height();
        let buf = &mut self.buffers[idx];
        let w = self.windows.selected_mut();
        w.view.page_down(buf, rows);
    }

    pub fn page_up_current(&mut self) {
        let id = self.windows.selected_buffer();
        let idx = self.buffer_index(id);
        let rows = self.selected_window_height();
        let buf = &mut self.buffers[idx];
        let w = self.windows.selected_mut();
        w.view.page_up(buf, rows);
    }

    pub fn recenter_current(&mut self) {
        let id = self.windows.selected_buffer();
        let idx = self.buffer_index(id);
        let rows = self.selected_window_height();
        let buf = &self.buffers[idx];
        let w = self.windows.selected_mut();
        w.view.recenter(buf, rows);
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
            self.buf_mut().undo_boundary();
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

    // --- isearch -----------------------------------------------------------

    pub fn isearch(&self) -> Option<&ISearch> {
        self.isearch.as_ref()
    }

    pub fn isearch_active(&self) -> bool {
        self.isearch.is_some()
    }

    pub fn set_isearch(&mut self, is: Option<ISearch>) {
        self.isearch = is;
    }

    pub fn take_isearch(&mut self) -> Option<ISearch> {
        self.isearch.take()
    }

    pub fn start_isearch(&mut self, forward: bool) {
        let point = self.buf().point();
        self.isearch = Some(ISearch::new(forward, point));
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

    /// Convenience: path of the selected window's buffer.
    pub fn current_buf_path(&self) -> Option<PathBuf> {
        self.buf().path().map(|p| p.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_arg_values() {
        assert_eq!(PrefixArg::default().value(), 1);
        assert_eq!(
            PrefixArg {
                universal: 1,
                ..Default::default()
            }
            .value(),
            4
        );
        assert_eq!(
            PrefixArg {
                universal: 2,
                ..Default::default()
            }
            .value(),
            16
        );
        assert_eq!(
            PrefixArg {
                digits: Some(12),
                ..Default::default()
            }
            .value(),
            12
        );
        assert_eq!(
            PrefixArg {
                digits: Some(3),
                negative: true,
                ..Default::default()
            }
            .value(),
            -3
        );
        assert_eq!(
            PrefixArg {
                negative: true,
                ..Default::default()
            }
            .value(),
            -1
        );
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

    #[test]
    fn split_windows_share_buffer() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("abc");
        let id = ed.selected_buffer_id();
        ed.split_window(crate::window::Split::Vertical);
        assert_eq!(ed.window_layout().len(), 2);
        assert_eq!(ed.selected_buffer_id(), id);
        assert!(!ed.single_window());
    }

    #[test]
    fn window_point_preserved() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("hello world");
        ed.buf_mut().move_to_buffer_start();
        ed.buf_mut().move_char(crate::buffer::Direction::Forward);
        ed.split_window(crate::window::Split::Vertical);
        // new window starts at the shared point
        assert_eq!(ed.buf().point(), 1);
        ed.buf_mut().move_char(crate::buffer::Direction::Forward);
        assert_eq!(ed.buf().point(), 2);
        assert!(ed.other_window());
        assert_eq!(ed.buf().point(), 1, "old window keeps its point");
        assert!(ed.other_window());
        assert_eq!(ed.buf().point(), 2);
    }
}
