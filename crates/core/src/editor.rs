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
    /// Accepted minibuffer inputs, for C-n/C-p history recall.
    minibuffer_history: Vec<String>,
    /// What the top suspended command coroutine is waiting for.
    pending: Option<PendingRequest>,
    /// Requests belonging to outer coroutines while a nested read-key loop
    /// temporarily owns input.
    pending_stack: Vec<PendingRequest>,
    /// Keys of the key sequence in progress (for prefix resolution).
    pending_keys: Vec<Key>,
    /// Esc acts as a Meta prefix (ESC x == M-x).
    esc_prefix: bool,
    quit: bool,
    /// Set by `suspend-frame` (C-z): the event loop hands the terminal
    /// back to the shell and suspends the process.
    suspend: bool,
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
    /// An overriding keymap, checked before the normal maps by the
    /// read-key loop of modes like isearch (Emacs's
    /// `overriding-terminal-local-map`).  It is stored and queried through
    /// `lookup_overriding_key`, separately from the normal dispatch.
    overriding_keymap: Option<Keymap>,
    /// The most recent key delivered to a `read-key` coroutine, for replay.
    replay_key: Option<Key>,
    /// The (start, end) char offsets of the current isearch match, if the
    /// search is active and found something (highlighted by the UI).
    search_match: Option<(usize, usize)>,
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
            minibuffer_history: Vec::new(),
            pending: None,
            pending_stack: Vec::new(),
            pending_keys: Vec::new(),
            esc_prefix: false,
            quit: false,
            suspend: false,
            window_rows,
            window_cols,
            script: None,
            mode_defs: std::collections::HashMap::new(),
            minor_defs: std::collections::HashMap::new(),
            pending_keymap: None,
            overriding_keymap: None,
            replay_key: None,
            search_match: None,
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
        self.buffer_index_opt(id).expect("buffer id exists")
    }

    /// Fallible buffer lookup for IDs supplied by Lua or file-watch events.
    pub fn buffer_index_opt(&self, id: usize) -> Option<usize> {
        self.buffers.iter().position(|b| b.id == id)
    }

    /// Index of the buffer editing commands act on: the minibuffer input
    /// while a read is in progress (it is a real registered buffer), else
    /// the buffer shown in the selected window (Emacs's model — the
    /// minibuffer is the current buffer during a read).
    pub fn current_buffer_index(&self) -> usize {
        if let Some(mb) = &self.minibuffer {
            return self.buffer_index(mb.buffer_id);
        }
        self.selected_buffer_index()
    }

    pub fn buf(&self) -> &Buffer {
        &self.buffers[self.current_buffer_index()]
    }

    pub fn buf_mut(&mut self) -> &mut Buffer {
        let idx = self.current_buffer_index();
        &mut self.buffers[idx]
    }

    /// Id of the buffer that editing commands act on (the minibuffer input
    /// while a read is in progress).
    pub fn current_buffer_id(&self) -> usize {
        if let Some(mb) = &self.minibuffer {
            return mb.buffer_id;
        }
        self.windows.selected_buffer()
    }

    /// The minibuffer input buffer, if a read is in progress.
    pub fn minibuffer_buffer(&self) -> Option<&Buffer> {
        self.minibuffer
            .as_ref()
            .map(|mb| &self.buffers[self.buffer_index(mb.buffer_id)])
    }

    /// The minibuffer state together with its input buffer, for editing.
    pub fn minibuffer_and_buffer_mut(&mut self) -> Option<(&mut Minibuffer, &mut Buffer)> {
        let mb = self.minibuffer.as_mut()?;
        let idx = self.buffers.iter().position(|b| b.id == mb.buffer_id)?;
        Some((mb, &mut self.buffers[idx]))
    }

    /// The minibuffer input text ("" when no read is active).
    pub fn minibuffer_input(&self) -> String {
        match self.minibuffer_buffer() {
            Some(buf) => buf.rope().to_string(),
            None => String::new(),
        }
    }

    /// The text RET accepts: the input plus the completion preview.
    pub fn minibuffer_accepted(&self) -> String {
        match (&self.minibuffer, self.minibuffer_buffer()) {
            (Some(mb), Some(buf)) => mb.accepted(buf),
            _ => String::new(),
        }
    }

    /// Display column of the minibuffer input cursor.
    pub fn minibuffer_cursor_col(&self) -> usize {
        match (&self.minibuffer, self.minibuffer_buffer()) {
            (Some(mb), Some(buf)) => mb.cursor_col(buf),
            _ => 0,
        }
    }

    pub fn minibuffer_insert_char(&mut self, c: char) {
        if let Some((mb, buf)) = self.minibuffer_and_buffer_mut() {
            mb.insert_char(buf, c);
        }
    }

    pub fn minibuffer_accept_preview(&mut self) {
        if let Some((mb, buf)) = self.minibuffer_and_buffer_mut() {
            mb.accept_preview(buf);
        }
    }

    pub fn minibuffer_cycle(&mut self) -> bool {
        match self.minibuffer_and_buffer_mut() {
            Some((mb, buf)) => mb.cycle(buf),
            None => false,
        }
    }

    pub fn minibuffer_complete_with(&mut self, candidates: Vec<String>, fill: bool) {
        if let Some((mb, buf)) = self.minibuffer_and_buffer_mut() {
            mb.complete_with(buf, candidates, fill);
        }
    }

    pub fn selected_buffer_id(&self) -> usize {
        self.windows.selected_buffer()
    }

    pub fn selected_buffer_index(&self) -> usize {
        let id = self.windows.selected_buffer();
        self.buffer_index(id)
    }

    /// Show `id` in the selected window, preserving window-points. The
    /// window's scroll state is reset: it belonged to the previous buffer.
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
        w.view.reset();
        let saved = w.point.take();
        if let Some(p) = saved {
            self.buffers[new_idx].set_point(p);
        }
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
    pub fn kill_buffer_at(&mut self, id: usize) -> Result<()> {
        if self
            .minibuffer
            .as_ref()
            .is_some_and(|mb| mb.buffer_id == id)
        {
            return Err(anyhow!("cannot kill the active minibuffer"));
        }
        let idx = self
            .buffer_index_opt(id)
            .ok_or_else(|| anyhow!("no buffer with id {id}"))?;
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
        Ok(())
    }

    /// Write the buffer with `id` to its file (raises on IO error; the Lua
    /// layer handles modified flags and save hooks).
    pub fn save_buffer_to_disk(&mut self, id: usize) -> Result<()> {
        let idx = self
            .buffer_index_opt(id)
            .ok_or_else(|| anyhow!("no buffer with id {id}"))?;
        self.buffers()[idx].save().map_err(|e| anyhow!("{e}"))
    }

    /// Replace the current buffer's content with `text` (used by dired
    /// listings and the *Help* buffer), leaving point at the start and the
    /// buffer unmodified.
    /// Replace the content of the buffer with `id` (used by dired listings
    /// and the *Help* buffer), leaving point at the start and the buffer
    /// unmodified.
    pub fn replace_buffer_content(&mut self, id: usize, text: &str) {
        let idx = self.buffer_index_opt(id).expect("buffer id exists");
        self.buffers[idx].replace_content(text);
    }

    // --- windows -----------------------------------------------------------

    /// The divider lines between split windows (window boundaries).
    pub fn window_dividers(&self) -> Vec<crate::window::Divider> {
        self.windows.dividers(self.body_rect())
    }

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

    /// Width of the selected window, for line wrapping and scrolling.
    pub fn selected_window_width(&self) -> usize {
        self.window_layout()
            .into_iter()
            .find(|l| l.selected)
            .map(|l| l.rect.w as usize)
            .unwrap_or(self.window_cols)
            .max(1)
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

    pub fn set_window_size(&mut self, rows: usize, cols: usize) {
        self.window_rows = rows;
        self.window_cols = cols;
    }

    /// Keep every window's view scrolled so its point stays visible. All
    /// windows scroll: a width change (resize, C-x 3) shifts the visual
    /// rows of even unselected windows.
    pub fn scroll_current_view(&mut self) {
        let body = self.body_rect();
        let selected_path = self.windows.selected_path().to_vec();
        let items: Vec<(Vec<usize>, Option<usize>, usize, crate::window::Rect)> = self
            .windows
            .layout(body)
            .into_iter()
            .map(|(path, w, rect)| (path, w.point, w.buffer, rect))
            .collect();
        for (path, saved, buffer, rect) in items {
            let selected = path == selected_path;
            let idx = self.buffer_index(buffer);
            let point = if selected {
                self.buffers[idx].point()
            } else {
                saved.unwrap_or(self.buffers[idx].point())
            };
            let buf = &self.buffers[idx];
            let w = self.windows.leaf_mut(&path);
            w.view
                .scroll_to(buf, rect.w.max(1) as usize, rect.h.max(1) as usize, point);
        }
    }

    pub fn page_down_current(&mut self) {
        let id = self.windows.selected_buffer();
        let idx = self.buffer_index(id);
        let rows = self.selected_window_height();
        let cols = self.selected_window_width();
        let buf = &mut self.buffers[idx];
        let w = self.windows.selected_mut();
        w.view.page_down(buf, cols, rows);
    }

    pub fn page_up_current(&mut self) {
        let id = self.windows.selected_buffer();
        let idx = self.buffer_index(id);
        let rows = self.selected_window_height();
        let cols = self.selected_window_width();
        let buf = &mut self.buffers[idx];
        let w = self.windows.selected_mut();
        w.view.page_up(buf, cols, rows);
    }

    pub fn recenter_current(&mut self) {
        let id = self.windows.selected_buffer();
        let idx = self.buffer_index(id);
        let rows = self.selected_window_height();
        let cols = self.selected_window_width();
        let buf = &self.buffers[idx];
        let w = self.windows.selected_mut();
        w.view.recenter(buf, cols, rows);
    }

    // --- search ------------------------------------------------------------

    /// Case-insensitive match of `query` at or after `from`; returns the
    /// (start, end) char offsets of the match (used by the Lua isearch
    /// implementation, which highlights the current match).
    pub fn search_forward(&self, query: &str, from: usize) -> Option<(usize, usize)> {
        crate::search::find_forward(self.buf().rope(), query, from)
    }

    /// Case-insensitive match of `query` strictly before `from`; returns
    /// the (start, end) char offsets of the match.
    pub fn search_backward(&self, query: &str, from: usize) -> Option<(usize, usize)> {
        crate::search::find_backward(self.buf().rope(), query, from)
    }

    /// The current isearch match to highlight (set/cleared by the Lua
    /// isearch implementation).
    pub fn search_match(&self) -> Option<(usize, usize)> {
        self.search_match
    }

    pub fn set_search_match(&mut self, m: Option<(usize, usize)>) {
        self.search_match = m;
    }

    // --- keymap / bindings -------------------------------------------------

    pub fn keymap_mut(&mut self) -> &mut Keymap {
        &mut self.keymap
    }

    /// Look up a key sequence against the active keymaps: enabled minor mode
    /// keymaps (most recently enabled first), the current buffer's local
    /// keymap, then the global keymap.  The current buffer is the minibuffer
    /// while a read is active, so its `minibuffer-mode` local keymap is
    /// consulted then.  A prefix result fixes the source for the rest of the
    /// sequence.
    pub fn lookup_key(&mut self, seq: &[Key]) -> Lookup {
        if seq.is_empty() {
            return Lookup::Unbound;
        }
        if let Some(src) = self.pending_keymap {
            let map = self.keymap_for_source(src);
            return map.lookup(seq);
        }
        let idx = self.current_buffer_index();
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

    /// Install the keymap a `read-key` loop consults before the normal
    /// maps (Emacs's `overriding-terminal-local-map`; isearch uses it).
    pub fn set_overriding_keymap(&mut self, keymap: Keymap) {
        self.overriding_keymap = Some(keymap);
    }

    /// Remove the overriding keymap (the read-key loop finished).
    pub fn clear_overriding_keymap(&mut self) {
        self.overriding_keymap = None;
    }

    /// Look a sequence up in the overriding keymap only.  Unlike
    /// `lookup_key`, this never touches the pending-prefix state: the
    /// read-key loop that owns the override tracks its own keys.
    pub fn lookup_overriding_key(&self, seq: &[Key]) -> Lookup {
        match &self.overriding_keymap {
            Some(map) if !seq.is_empty() => map.lookup(seq),
            _ => Lookup::Unbound,
        }
    }

    fn keymap_for_source(&self, src: KeymapSource) -> &Keymap {
        match src {
            KeymapSource::Global => &self.keymap,
            KeymapSource::Local => self.buffers[self.current_buffer_index()]
                .local_keymap()
                .expect("local keymap exists"),
            KeymapSource::Minor(i) => {
                let idx = self.current_buffer_index();
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
        let idx = self.current_buffer_index();
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
    pub fn local_set_key(&mut self, idx: usize, seq: &[Key], cmd: &str) -> Result<()> {
        self.buffers[idx]
            .local_keymap_mut()
            .bind_sequence(seq, cmd)
            .map_err(|e| anyhow!(e))
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

    /// `suspend-frame` (C-z): the event loop suspends the process and
    /// hands the terminal back to the shell.
    pub fn set_suspend(&mut self) {
        self.suspend = true;
    }

    pub fn suspend_requested(&self) -> bool {
        self.suspend
    }

    pub fn clear_suspend_request(&mut self) {
        self.suspend = false;
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

    /// Record an accepted minibuffer input in the history (consecutive
    /// duplicates collapse).
    pub fn push_minibuffer_history(&mut self, entry: String) {
        if entry.is_empty() {
            return;
        }
        if self.minibuffer_history.last() != Some(&entry) {
            self.minibuffer_history.push(entry);
        }
    }

    /// Kill the minibuffer input buffer (if any) and drop its state.  Does
    /// not touch the pending request; [`Editor::end_pending_read`] ends a
    /// read as a whole.
    fn end_minibuffer(&mut self) {
        if let Some(old) = self.minibuffer.take() {
            let _ = self.kill_buffer_at(old.buffer_id);
        }
    }

    /// End the active pending read: its input has been answered, so the
    /// minibuffer input buffer (if any) is killed and the pending request
    /// cleared.  A command finishing (`finish_command`) never ends a read
    /// by itself.
    pub fn end_pending_read(&mut self) {
        let Some(request) = self.pending.take() else {
            return;
        };
        if matches!(
            request,
            PendingRequest::ReadString { .. } | PendingRequest::ReadYesNo { .. }
        ) {
            self.end_minibuffer();
        }
        self.pending = self.pending_stack.pop();
    }

    /// C-n / C-p: step through the input history, recalling entries into
    /// the minibuffer input. Past the last entry the input is empty.
    pub fn minibuffer_history_step(&mut self, dir: isize) {
        let Some(history_index) = self.minibuffer.as_ref().map(|mb| mb.history_index) else {
            return;
        };
        let len = self.minibuffer_history.len() as isize;
        let idx = history_index as isize + dir;
        if !(0..=len).contains(&idx) {
            return;
        }
        let text = if (idx as usize) < self.minibuffer_history.len() {
            self.minibuffer_history[idx as usize].clone()
        } else {
            String::new()
        };
        if let Some((mb, buf)) = self.minibuffer_and_buffer_mut() {
            mb.history_index = idx as usize;
            mb.set_input(buf, &text);
        }
    }

    pub fn pending(&self) -> Option<&PendingRequest> {
        self.pending.as_ref()
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

    /// Apply a command outcome.  `Done` only extracts a requested replay
    /// key: it never ends a pending read (a read ends when its coroutine
    /// is resumed with a value, [`Editor::end_pending_read`]), so a command
    /// run from the minibuffer leaves the read in place.  A pending request
    /// replaces any active one (a nested read).
    pub fn finish_command(&mut self, outcome: CommandOutcome) -> Option<Key> {
        match outcome {
            CommandOutcome::Done => {
                let key = if self.replay {
                    self.replay_key.take()
                } else {
                    None
                };
                self.replay = false;
                self.replay_key = None;
                key
            }
            CommandOutcome::Pending(PendingRequest::ReadString {
                prompt,
                completion,
                initial,
            }) => {
                // A nested read replaces the outer minibuffer (if any).
                self.end_minibuffer();
                // The input is a real, registered buffer (Emacs's model):
                // the minibuffer is the current buffer during the read.
                let mut buf = Buffer::new(" *Minibuf*");
                if !initial.is_empty() {
                    buf.insert(&initial);
                    buf.move_to_buffer_end();
                    // the pre-filled input is not a user modification
                    buf.set_modified(false);
                }
                let id = buf.id;
                self.buffers.push(buf);
                if self.mode_defs.contains_key("minibuffer-mode") {
                    let idx = self.buffers.len() - 1;
                    let _ = self.set_buffer_mode_by_name(idx, "minibuffer-mode");
                }
                let mut mb = Minibuffer::new(id, prompt.clone(), completion, Some(initial.clone()));
                mb.history_index = self.minibuffer_history.len();
                self.minibuffer = Some(mb);
                self.pending = Some(PendingRequest::ReadString {
                    prompt,
                    completion,
                    initial,
                });
                None
            }
            CommandOutcome::Pending(p) => {
                if matches!(p, PendingRequest::ReadKey) && self.pending.is_some() {
                    if let Some(outer) = self.pending.take() {
                        self.pending_stack.push(outer);
                    }
                } else {
                    self.end_minibuffer();
                }
                self.pending = Some(p);
                None
            }
        }
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

    /// Load the required runtime modules embedded in the binary, in the
    /// order the scripting engine defines.
    pub fn load_runtime(&mut self) -> Result<()> {
        let mut res = Ok(());
        self.with_host(|ed, host| {
            res = host.load_runtime(ed);
        });
        res
    }

    /// Load the required runtime modules from a directory instead.
    pub fn load_runtime_dir(&mut self, dir: &Path) -> Result<()> {
        let mut res = Ok(());
        self.with_host(|ed, host| {
            res = host.load_runtime_dir(dir, ed);
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

    /// Resume a suspended command coroutine.  The pending read ends first
    /// (`end_pending_read`): the minibuffer input buffer is killed and the
    /// continuation runs against the main buffer (Emacs:
    /// `read-from-minibuffer` returns, then the caller continues).
    pub fn resume_pending(&mut self, value: ResumeValue) -> Result<CommandOutcome> {
        let Some(request) = self.pending.as_ref() else {
            let mut out = Err(anyhow!("no script host attached"));
            self.with_host(|ed, host| {
                out = host.resume_pending(value, ed);
            });
            return out;
        };
        if !request.accepts(&value) {
            return Err(anyhow!("resume value does not match pending request"));
        }
        let is_minibuffer = matches!(
            request,
            PendingRequest::ReadString { .. } | PendingRequest::ReadYesNo { .. }
        );
        if is_minibuffer {
            self.end_minibuffer();
        }
        self.pending = self.pending_stack.pop();
        let mut out = Err(anyhow!("no script host attached"));
        self.with_host(|ed, host| {
            out = host.resume_pending(value, ed);
        });
        out
    }

    /// C-g deactivates the mark of the current buffer (quitting cancels the
    /// active region; the mark position is kept, so C-x C-x can reactivate
    /// it).
    pub fn deactivate_mark(&mut self) {
        let idx = self.current_buffer_index();
        self.buffers[idx].deactivate_mark();
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

    /// Notify the scripting host that the files of the given buffers
    /// changed on disk (inotify events); the Lua side applies the
    /// auto-revert policy.
    pub fn notify_file_changes(&mut self, ids: &[usize]) -> Result<()> {
        let mut res = Ok(());
        self.with_host(|ed, host| {
            res = host.notify_file_changes(ids, ed);
        });
        res
    }

    /// Re-read the file of the buffer with `id` from disk.
    pub fn reload_buffer_from_disk(&mut self, id: usize) -> Result<()> {
        let idx = self
            .buffer_index_opt(id)
            .ok_or_else(|| anyhow!("no buffer with id {id}"))?;
        self.buffers[idx]
            .reload_from_disk()
            .map_err(|e| anyhow!("{e}"))
    }

    // --- major / minor modes ----------------------------------------------

    pub fn register_mode_def(&mut self, def: ModeDef) {
        self.mode_defs.insert(def.name.clone(), def);
    }

    /// Set the major mode of the selected buffer from a registered
    /// Set the major mode of the selected buffer from a registered
    /// definition, installing its local keymap.
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
    fn overriding_keymap_is_queried_separately() {
        let mut ed = Editor::new(20, 80);
        let mut km = Keymap::new();
        km.bind_sequence(
            &crate::key::parse_sequence("C-s").unwrap(),
            "isearch-repeat-forward",
        )
        .unwrap();
        ed.set_overriding_keymap(km);
        assert_eq!(
            ed.lookup_overriding_key(&crate::key::parse_sequence("C-s").unwrap()),
            Lookup::Command("isearch-repeat-forward".into())
        );
        assert_eq!(
            ed.lookup_overriding_key(&crate::key::parse_sequence("C-n").unwrap()),
            Lookup::Unbound
        );
        // normal lookup does not see the override
        assert_eq!(
            ed.lookup_key(&crate::key::parse_sequence("C-s").unwrap()),
            Lookup::Unbound
        );
        ed.clear_overriding_keymap();
        assert_eq!(
            ed.lookup_overriding_key(&crate::key::parse_sequence("C-s").unwrap()),
            Lookup::Unbound
        );
    }

    #[test]
    fn local_keymap_overrides_global() {
        let mut ed = Editor::new(20, 80);
        ed.keymap_mut()
            .bind_sequence(&crate::key::parse_sequence("C-f").unwrap(), "forward-char")
            .unwrap();
        let idx = ed.selected_buffer_index();
        ed.local_set_key(
            idx,
            &crate::key::parse_sequence("C-f").unwrap(),
            "beginning-of-buffer",
        )
        .unwrap();
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
            .bind_sequence(&crate::key::parse_sequence("C-f").unwrap(), "forward-char")
            .unwrap();
        let mut km = Keymap::new();
        km.bind(Key::ctrl('f'), "end-of-buffer").unwrap();
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
        local
            .bind_sequence(&crate::key::parse_sequence("C-c C-c").unwrap(), "local-cmd")
            .unwrap();
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
            initial: String::new(),
        }));
        assert!(key.is_none());
        assert!(ed.minibuffer().is_some());
        assert!(matches!(
            ed.pending(),
            Some(PendingRequest::ReadString { .. })
        ));
        assert!(ed.minibuffer().unwrap().completion);
        let mb_id = ed.minibuffer().unwrap().buffer_id;
        // A finished nested command does NOT end the read ...
        let key = ed.finish_command(CommandOutcome::Done);
        assert!(key.is_none());
        assert!(ed.minibuffer().is_some(), "Done does not end a read");
        assert!(matches!(
            ed.pending(),
            Some(PendingRequest::ReadString { .. })
        ));
        // ... only answering it (end_pending_read) does; the input buffer
        // is a real registered buffer and gets killed.
        ed.end_pending_read();
        assert!(ed.minibuffer().is_none());
        assert!(ed.pending().is_none());
        assert!(!ed.buffers().iter().any(|b| b.id == mb_id));
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
    fn minibuffer_history_recalls_entries() {
        let mut ed = Editor::new(20, 80);
        ed.push_minibuffer_history("first".into());
        ed.push_minibuffer_history("second".into());
        ed.push_minibuffer_history("second".into()); // consecutive dup collapses
        assert_eq!(ed.minibuffer_history.len(), 2);

        ed.finish_command(CommandOutcome::Pending(PendingRequest::ReadString {
            prompt: "M-x ".into(),
            completion: true,
            initial: String::new(),
        }));
        // C-p recalls the most recent entry, then the one before it
        ed.minibuffer_history_step(-1);
        assert_eq!(ed.minibuffer_input(), "second");
        ed.minibuffer_history_step(-1);
        assert_eq!(ed.minibuffer_input(), "first");
        // at the oldest entry, C-p stops
        ed.minibuffer_history_step(-1);
        assert_eq!(ed.minibuffer_input(), "first");
        // C-n walks forward, past the end the input is empty
        ed.minibuffer_history_step(1);
        ed.minibuffer_history_step(1);
        assert_eq!(ed.minibuffer_input(), "");
        // editing after recall inserts into the recalled text
        ed.minibuffer_history_step(-1);
        ed.minibuffer_insert_char('!');
        assert_eq!(ed.minibuffer_input(), "second!");
    }

    #[test]
    fn search_forward_uses_rope() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("hello world hello");
        assert_eq!(ed.search_forward("hello", 0), Some((0, 5)));
        assert_eq!(ed.search_forward("hello", 1), Some((12, 17)));
        assert_eq!(ed.search_backward("hello", 11), Some((0, 5)));
    }
}
