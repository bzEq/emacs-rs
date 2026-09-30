//! The Editor: the minimal Rust core — buffers, windows, keymaps, echo
//! area, minibuffer input state, and the interface to the scripting host.
//! Commands, modes' behavior, undo, the kill ring, isearch, and dired all
//! live in Lua on top of these primitives.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::buffer::Buffer;
use crate::key::Key;
use crate::keymap::{Keymap, Lookup};
use crate::minibuffer::Minibuffer;
use crate::minor::MinorModeDef;
use crate::mode::ModeDef;
use crate::script::{CommandOutcome, NullHost, PendingRequest, ResumeValue, ScriptHost};
use crate::view::View;
use crate::window::{Rect as WinRect, Split, WindowTree};

/// Which keymap resolved the key sequence in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeymapSource {
    Global,
    /// The buffer's local (major mode) keymap.
    Local,
    /// An enabled minor mode keymap; index into the buffer's enabled list,
    /// 0 = most recently enabled.
    Minor(usize),
}

/// One window's rendering info, handed to the UI.
pub struct WindowLayout<'a> {
    pub buf: &'a Buffer,
    pub view: &'a View,
    pub rect: WinRect,
    pub selected: bool,
}

/// Snapshot of a buffer's management fields, for the Lua side.
pub struct BufferInfo<'a> {
    pub id: usize,
    pub name: &'a str,
    pub path: Option<&'a Path>,
    pub modified: bool,
    pub read_only: bool,
}

/// One enabled minor mode's keymap, for describe-bindings.
pub type MinorBindingSet = (String, String, Vec<(Vec<Key>, String)>);

pub struct Editor {
    buffers: Vec<Buffer>,
    windows: WindowTree,
    keymap: Keymap,
    /// Message in the echo area, if any.
    echo: Option<String>,
    /// True if `echo` is an error.
    echo_error: bool,
    minibuffer: Option<Minibuffer>,
    /// What a suspended command coroutine is waiting for.
    pending: Option<PendingRequest>,
    /// Keys of the key sequence in progress (for prefix resolution).
    pending_keys: Vec<Key>,
    /// Esc acts as a Meta prefix (ESC x == M-x).
    esc_prefix: bool,
    quit: bool,
    /// Rows/cols of the buffer area (terminal size minus modeline + echo).
    window_rows: usize,
    window_cols: usize,
    script: Option<Box<dyn ScriptHost>>,
    /// Major mode definitions by name (Lua-defined).
    mode_defs: std::collections::HashMap<String, ModeDef>,
    /// Minor mode definitions by name (Lua-defined).
    minor_defs: std::collections::HashMap<String, MinorModeDef>,
    /// Which keymap the key sequence in progress belongs to.
    pending_keymap: Option<KeymapSource>,
    /// The most recent key delivered to a `read-key` coroutine, for replay.
    replay_key: Option<Key>,
    /// Set by the Lua isearch layer to replay `replay_key` after exit.
    replay: bool,
}

impl Editor {
    pub fn new(window_rows: usize, window_cols: usize) -> Self {
        let scratch = Buffer::new("*scratch*");
        let scratch_id = scratch.id;
        Editor {
            buffers: vec![scratch],
            windows: WindowTree::new(scratch_id),
            keymap: Keymap::new(),
            echo: None,
            echo_error: false,
            minibuffer: None,
            pending: None,
            pending_keys: Vec::new(),
            esc_prefix: false,
            quit: false,
            window_rows,
            window_cols,
            script: None,
            mode_defs: std::collections::HashMap::new(),
            minor_defs: std::collections::HashMap::new(),
            pending_keymap: None,
            replay_key: None,
            replay: false,
        }
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
            return Err(anyhow!("no buffer named {name}"));
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

    /// Create a new, empty buffer and return its id.
    pub fn new_buffer(&mut self, name: &str) -> usize {
        let buf = Buffer::new(name.to_string());
        let id = buf.id;
        self.buffers.push(buf);
        id
    }

    /// Remove the buffer at `idx` (caller handles windows).
    pub fn remove_buffer(&mut self, idx: usize) {
        self.buffers.remove(idx);
    }

    /// Point all windows showing `old_id` at `new_id` (buffer killed).
    pub fn replace_buffer_in_windows(&mut self, old_id: usize, new_id: usize) {
        self.windows.replace_buffer(old_id, new_id);
    }

    /// Kill the buffer with id `id`, pointing any windows that displayed it
    /// at another buffer.
    pub fn kill_buffer_at(&mut self, id: usize) {
        let idx = self.buffer_index(id);
        self.remove_buffer(idx);
        if self.buffers().is_empty() {
            let scratch = Buffer::new("*scratch*");
            let sid = scratch.id;
            self.add_buffer(scratch);
            self.replace_buffer_in_windows(id, sid);
        } else {
            let keep_id = self.buffers()[0].id;
            self.replace_buffer_in_windows(id, keep_id);
        }
    }

    /// Management fields of every buffer, for the Lua side.
    pub fn buffer_infos(&self) -> Vec<BufferInfo<'_>> {
        self.buffers
            .iter()
            .map(|b| BufferInfo {
                id: b.id,
                name: b.name(),
                path: b.path(),
                modified: b.modified(),
                read_only: b.read_only(),
            })
            .collect()
    }

    /// Write the buffer with `id` to its file (raises on IO error; the Lua
    /// layer handles modified flags and save hooks).
    pub fn save_buffer_to_disk(&mut self, id: usize) -> Result<()> {
        let idx = self.buffer_index(id);
        self.buffers()[idx].save().map_err(|e| anyhow!("{e}"))
    }

    /// Replace the current buffer's content with `text` (used by dired
    /// listings and the *Help* buffer), leaving point at the start and the
    /// buffer unmodified.
    pub fn replace_buffer_content(&mut self, text: &str) {
        let len = self.buf().rope().len_chars();
        if len > 0 {
            let _ = self.buf_mut().delete_range(0, len);
        }
        self.buf_mut().set_point(0);
        self.buf_mut().insert(text);
        self.buf_mut().set_point(0);
        self.buf_mut().set_modified(false);
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

    /// Re-parse the selected window's buffer for highlighting if it has a
    /// language mode and is marked dirty (or has no tree yet). Files above
    /// the parse caps are left alone, and re-parses are throttled, to keep
    /// large-file editing snappy.
    pub fn refresh_syntax_current(&mut self) {
        const COOLDOWN: std::time::Duration = std::time::Duration::from_millis(200);
        let idx = self.selected_buffer_index();
        let buf = &mut self.buffers[idx];
        let Some(lang) = buf.mode().lang else {
            return;
        };
        let len = buf.rope().len_chars();
        if len > crate::syntax::MAX_PARSE_CHARS {
            return;
        }
        if buf.syntax().is_some() && len > crate::syntax::MAX_REPARSE_CHARS {
            return; // keep the initial parse
        }
        if !buf.syntax_dirty() && buf.syntax().is_some() {
            return;
        }
        if buf.syntax().is_some()
            && buf
                .syntax_last_parse()
                .is_some_and(|t| t.elapsed() < COOLDOWN)
        {
            return; // dirty but throttled; re-parsed on a later key
        }
        let text = buf.rope().to_string();
        if let Some(s) = crate::syntax::parse(lang, &text) {
            buf.set_syntax(Some(s));
            buf.set_syntax_dirty(false);
            buf.set_syntax_last_parse(std::time::Instant::now());
        }
    }

    // --- search ------------------------------------------------------------

    /// Case-insensitive match of `query` at or after `from`, in char
    /// offsets (used by the Lua isearch implementation).
    pub fn search_forward(&self, query: &str, from: usize) -> Option<usize> {
        crate::search::find_forward(self.buf().rope(), query, from)
    }

    /// Case-insensitive match of `query` strictly before `from`.
    pub fn search_backward(&self, query: &str, from: usize) -> Option<usize> {
        crate::search::find_backward(self.buf().rope(), query, from)
    }

    // --- keymap / bindings -------------------------------------------------

    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    pub fn keymap_mut(&mut self) -> &mut Keymap {
        &mut self.keymap
    }

    /// Look up a key sequence against the active keymaps: enabled minor mode
    /// keymaps (most recently enabled first), the buffer's local keymap,
    /// then the global keymap. A prefix result fixes the source for the
    /// rest of the sequence.
    pub fn lookup_key(&mut self, seq: &[Key]) -> Lookup {
        if seq.is_empty() {
            return Lookup::Unbound;
        }
        if let Some(src) = self.pending_keymap {
            let map = self.keymap_for_source(src);
            return map.lookup(seq);
        }
        let idx = self.selected_buffer_index();
        let minor_sources: Vec<KeymapSource> = self.buffers[idx]
            .enabled_minor()
            .iter()
            .rev()
            .enumerate()
            .filter_map(|(i, name)| {
                self.minor_defs
                    .get(name)
                    .and_then(|d| d.keymap.as_ref())
                    .map(|_| KeymapSource::Minor(i))
            })
            .collect();
        let mut sources: Vec<KeymapSource> = minor_sources;
        if self.buffers[idx].local_keymap().is_some() {
            sources.push(KeymapSource::Local);
        }
        sources.push(KeymapSource::Global);
        for src in sources {
            let map = self.keymap_for_source(src);
            match map.lookup(seq) {
                Lookup::Unbound => continue,
                Lookup::Prefix => {
                    self.pending_keymap = Some(src);
                    return Lookup::Prefix;
                }
                cmd => return cmd,
            }
        }
        Lookup::Unbound
    }

    fn keymap_for_source(&self, src: KeymapSource) -> &Keymap {
        match src {
            KeymapSource::Global => &self.keymap,
            KeymapSource::Local => self.buffers[self.selected_buffer_index()]
                .local_keymap()
                .expect("local keymap exists"),
            KeymapSource::Minor(i) => {
                let idx = self.selected_buffer_index();
                let name = self.buffers[idx]
                    .enabled_minor()
                    .iter()
                    .rev()
                    .nth(i)
                    .expect("minor mode exists");
                self.minor_defs[name]
                    .keymap
                    .as_ref()
                    .expect("minor keymap exists")
            }
        }
    }

    /// All global bindings, flattened and sorted, for describe-bindings.
    pub fn global_bindings(&self) -> Vec<(Vec<Key>, String)> {
        self.keymap.flatten()
    }

    /// The selected buffer's local keymap bindings.
    pub fn local_bindings(&self) -> Vec<(Vec<Key>, String)> {
        self.buf()
            .local_keymap()
            .map(Keymap::flatten)
            .unwrap_or_default()
    }

    /// (name, lighter, bindings) of every enabled minor mode with a keymap.
    pub fn minor_binding_sets(&self) -> Vec<MinorBindingSet> {
        let idx = self.selected_buffer_index();
        let mut out = Vec::new();
        for name in self.buffers[idx].enabled_minor().to_vec() {
            if let Some(def) = self.minor_defs.get(&name) {
                if let Some(km) = &def.keymap {
                    out.push((name.clone(), def.lighter.clone(), km.flatten()));
                }
            }
        }
        out
    }

    /// Add a binding to the selected buffer's local keymap, creating it if
    /// needed.
    pub fn local_set_key(&mut self, idx: usize, seq: &[Key], cmd: &str) {
        self.buffers[idx].local_keymap_mut().bind_sequence(seq, cmd);
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
        self.pending_keymap = None;
    }

    pub fn esc_prefix(&self) -> bool {
        self.esc_prefix
    }

    pub fn set_esc_prefix(&mut self, v: bool) {
        self.esc_prefix = v;
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

    pub fn pending(&self) -> Option<&PendingRequest> {
        self.pending.as_ref()
    }

    /// True if point (in the current buffer) is inside a comment or string
    /// node (used by electric-newline-and-maybe-indent in Lua).
    pub fn point_in_comment_or_string(&self) -> bool {
        let buf = self.buf();
        let Some(s) = buf.syntax() else {
            return false;
        };
        crate::syntax::point_in_comment_or_string(s, buf.rope(), buf.point())
    }

    /// Record the key that was delivered to a `read-key` request, so a
    /// command can ask to replay it after finishing.
    pub fn set_read_key(&mut self, key: Key) {
        self.replay_key = Some(key);
    }

    /// Called by the Lua layer: replay the pending key after the command
    /// coroutine finishes.
    pub fn set_replay(&mut self) {
        self.replay = true;
    }

    /// Apply a command outcome: clear the minibuffer, store the pending
    /// request (installing a fresh minibuffer for read-string requests).
    /// Returns a key to replay through the normal dispatch, if requested.
    pub fn finish_command(&mut self, outcome: CommandOutcome) -> Option<Key> {
        self.minibuffer = None;
        match outcome {
            CommandOutcome::Done => {
                self.pending = None;
                let key = if self.replay {
                    self.replay_key.take()
                } else {
                    None
                };
                self.replay = false;
                self.replay_key = None;
                key
            }
            CommandOutcome::Pending(PendingRequest::ReadString { prompt, completion }) => {
                self.minibuffer = Some(Minibuffer::new(prompt.clone(), completion));
                self.pending = Some(PendingRequest::ReadString { prompt, completion });
                None
            }
            CommandOutcome::Pending(p) => {
                self.pending = Some(p);
                None
            }
        }
    }

    /// Clear all input state (minibuffer, pending request, pending keys).
    pub fn abort_pending(&mut self) {
        self.pending = None;
        self.minibuffer = None;
        self.replay = false;
        self.replay_key = None;
        self.clear_pending_keys();
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

    /// Load a script file (a defaults module or the user init.lua).
    pub fn load_script(&mut self, path: &Path) -> Result<()> {
        let mut res = Ok(());
        self.with_host(|ed, host| {
            res = host.load_file(path, ed);
        });
        res
    }

    /// Load the required runtime modules from a directory, in the order the
    /// scripting engine defines.
    pub fn load_runtime(&mut self, dir: &Path) -> Result<()> {
        let mut res = Ok(());
        self.with_host(|ed, host| {
            res = host.load_runtime(dir, ed);
        });
        res
    }

    /// Invoke a command by name through the script host.
    pub fn call_command(&mut self, name: &str, extra: Option<char>) -> Result<CommandOutcome> {
        let mut out = Err(anyhow!("no script host attached"));
        self.with_host(|ed, host| {
            out = host.call_command(name, extra, ed);
        });
        out
    }

    /// Resume a suspended command coroutine.
    pub fn resume_pending(&mut self, value: ResumeValue) -> Result<CommandOutcome> {
        let mut out = Err(anyhow!("no script host attached"));
        self.with_host(|ed, host| {
            out = host.resume_pending(value, ed);
        });
        out
    }

    /// Recompute minibuffer completion candidates through the script host.
    pub fn update_completion(&mut self, input: &str) -> Result<Vec<String>> {
        let mut out = Ok(Vec::new());
        self.with_host(|ed, host| {
            out = host.update_completion(input, ed);
        });
        out
    }

    /// Run the Lua startup function with the command-line file argument.
    pub fn run_startup(&mut self, path: Option<&str>) -> Result<()> {
        let mut res = Ok(());
        self.with_host(|ed, host| {
            res = host.run_startup(path, ed);
        });
        res
    }

    // --- major / minor modes ----------------------------------------------

    pub fn register_mode_def(&mut self, def: ModeDef) {
        self.mode_defs.insert(def.name.clone(), def);
    }

    pub fn mode_def(&self, name: &str) -> Option<&ModeDef> {
        self.mode_defs.get(name)
    }

    /// Set the major mode of the selected buffer from a registered
    /// definition, installing its local keymap and re-parsing if the
    /// language changed.
    pub fn set_buffer_mode_by_name(&mut self, idx: usize, name: &str) -> Result<()> {
        let def = self
            .mode_defs
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow!("no major mode named {name}"))?;
        let buf = &mut self.buffers[idx];
        buf.set_mode(def.to_mode());
        let local = def.keymap.filter(|k| !k.is_empty());
        buf.set_local_keymap(local);
        let lang = def.lang;
        if lang.is_some() {
            buf.set_syntax(None);
            buf.set_syntax_dirty(true);
        } else {
            buf.set_syntax(None);
            buf.set_syntax_dirty(false);
        }
        Ok(())
    }

    pub fn register_minor_def(&mut self, def: MinorModeDef) {
        self.minor_defs.insert(def.name.clone(), def);
    }

    pub fn minor_def(&self, name: &str) -> Option<&MinorModeDef> {
        self.minor_defs.get(name)
    }

    /// Toggle a minor mode on the selected window's buffer; returns the new
    /// state (true = enabled).
    pub fn toggle_minor_mode(&mut self, idx: usize, name: &str) -> Result<bool> {
        if !self.minor_defs.contains_key(name) {
            return Err(anyhow!("no minor mode named {name}"));
        }
        let buf = &mut self.buffers[idx];
        if buf.minor_mode_enabled(name) {
            buf.disable_minor_mode(name);
            Ok(false)
        } else {
            buf.enable_minor_mode(name);
            Ok(true)
        }
    }

    pub fn set_minor_mode(&mut self, idx: usize, name: &str, enable: bool) -> Result<bool> {
        if !self.minor_defs.contains_key(name) {
            return Err(anyhow!("no minor mode named {name}"));
        }
        let buf = &mut self.buffers[idx];
        if enable {
            buf.enable_minor_mode(name);
        } else {
            buf.disable_minor_mode(name);
        }
        Ok(enable)
    }

    pub fn minor_mode_enabled(&self, idx: usize, name: &str) -> bool {
        self.buffers[idx].minor_mode_enabled(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn local_keymap_overrides_global() {
        let mut ed = Editor::new(20, 80);
        ed.keymap_mut()
            .bind_sequence(&crate::key::parse_sequence("C-f").unwrap(), "forward-char");
        let idx = ed.selected_buffer_index();
        ed.local_set_key(
            idx,
            &crate::key::parse_sequence("C-f").unwrap(),
            "beginning-of-buffer",
        );
        ed.push_key(Key::ctrl('f'));
        let seq = ed.pending_keys().to_vec();
        assert_eq!(
            ed.lookup_key(&seq),
            Lookup::Command("beginning-of-buffer".into())
        );
        ed.clear_pending_keys();
        ed.push_key(Key::ctrl('b'));
        let seq = ed.pending_keys().to_vec();
        assert_eq!(ed.lookup_key(&seq), Lookup::Unbound);
    }

    #[test]
    fn minor_keymap_has_priority() {
        let mut ed = Editor::new(20, 80);
        ed.keymap_mut()
            .bind_sequence(&crate::key::parse_sequence("C-f").unwrap(), "forward-char");
        let mut km = Keymap::new();
        km.bind(Key::ctrl('f'), "end-of-buffer");
        ed.register_minor_def(MinorModeDef {
            name: "test-minor".into(),
            doc: String::new(),
            lighter: "TM".into(),
            keymap: Some(km),
        });
        let idx = ed.selected_buffer_index();
        ed.toggle_minor_mode(idx, "test-minor").unwrap();
        ed.push_key(Key::ctrl('f'));
        let seq = ed.pending_keys().to_vec();
        assert_eq!(ed.lookup_key(&seq), Lookup::Command("end-of-buffer".into()));
        ed.toggle_minor_mode(idx, "test-minor").unwrap();
        ed.clear_pending_keys();
        ed.push_key(Key::ctrl('f'));
        let seq = ed.pending_keys().to_vec();
        assert_eq!(ed.lookup_key(&seq), Lookup::Command("forward-char".into()));
    }

    #[test]
    #[allow(clippy::unnecessary_to_owned)]
    fn prefix_stays_in_source_keymap() {
        let mut ed = Editor::new(20, 80);
        let mut local = Keymap::new();
        local.bind_sequence(&crate::key::parse_sequence("C-c C-c").unwrap(), "local-cmd");
        let idx = ed.selected_buffer_index();
        ed.buffers_mut()[idx].set_local_keymap(Some(local));
        ed.push_key(Key::ctrl('c'));
        // to_vec() avoids borrowing `ed` twice in the same call
        assert_eq!(ed.lookup_key(&ed.pending_keys().to_vec()), Lookup::Prefix);
        ed.push_key(Key::ctrl('c'));
        assert_eq!(
            ed.lookup_key(&ed.pending_keys().to_vec()),
            Lookup::Command("local-cmd".into())
        );
    }

    #[test]
    fn finish_command_installs_minibuffer() {
        let mut ed = Editor::new(20, 80);
        let key = ed.finish_command(CommandOutcome::Pending(PendingRequest::ReadString {
            prompt: "M-x ".into(),
            completion: true,
        }));
        assert!(key.is_none());
        assert!(ed.minibuffer().is_some());
        assert!(matches!(
            ed.pending(),
            Some(PendingRequest::ReadString { .. })
        ));
        assert!(ed.minibuffer().unwrap().completion);
        // Done clears both
        let key = ed.finish_command(CommandOutcome::Done);
        assert!(key.is_none());
        assert!(ed.minibuffer().is_none());
        assert!(ed.pending().is_none());
    }

    #[test]
    fn finish_command_returns_replay_key() {
        let mut ed = Editor::new(20, 80);
        ed.set_read_key(Key::ctrl('a'));
        ed.set_replay();
        let key = ed.finish_command(CommandOutcome::Done);
        assert_eq!(key, Some(Key::ctrl('a')));
        // replay key is consumed
        assert!(ed.finish_command(CommandOutcome::Done).is_none());
    }

    #[test]
    fn search_forward_uses_rope() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("hello world hello");
        assert_eq!(ed.search_forward("hello", 0), Some(0));
        assert_eq!(ed.search_forward("hello", 1), Some(12));
        assert_eq!(ed.search_backward("hello", 11), Some(0));
    }
}
