//! Rope-backed text buffer with Emacs-style cursor semantics.
//!
//! # One concrete buffer type (Emacs's model, trimmed to scope)
//!
//! Even Emacs has a single `struct buffer`: a "binary buffer" is just a
//! unibyte buffer, and the minibuffer, dired, *Help* and friends are
//! ordinary buffers specialized by buffer-local state and modes — not by
//! type. This port follows the single-type part: `Buffer` is one concrete
//! type (the minibuffer is a registered Buffer with `minibuffer-mode`),
//! and per-buffer behavior lives in the major mode, the local keymap, and
//! minor modes. A trait hierarchy (`TextBuffer`/`BinaryBuffer`/...) would
//! push dynamic dispatch through the whole core for no benefit here.
//!
//! emacs-rs is intentionally a small plain-text editor, not a full Emacs:
//! unibyte/raw-byte storage, buffer-local variables, and text
//! properties/overlays are out of scope, as are the subsystems built on
//! them (binary editing, `hexl-mode`, rich display). The Rust core stays
//! minimal; policy and optional features belong in Lua extensions.

use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ropey::Rope;

use crate::keymap::Keymap;
use crate::mode::{fundamental, Mode};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

/// An Emacs-like text buffer backed by a `ropey::Rope`.
///
/// Point is stored as a char offset into the rope. All edits are O(log n).
/// Vertical motion remembers the goal column (Emacs MOVE_TO_VAR semantics).
/// CRLF files are treated as LF: cursor motion steps over `\r\n` as a unit
/// and the trailing `\r` is invisible to point.
///
/// This is a *primitive* buffer: edits do not record undo information; the
/// Lua layer maintains undo lists on top of these primitives.
#[derive(Debug)]
pub struct Buffer {
    pub id: usize,
    name: String,
    path: Option<PathBuf>,
    rope: Rope,
    point: usize,
    /// Preferred column (in chars) for vertical motion, if set by a prior
    /// horizontal move.
    goal_column: Option<usize>,
    modified: bool,
    /// The mark position, if set (C-SPC). The position persists across
    /// deactivation (Emacs keeps the mark; C-x C-x can reactivate it).
    mark: Option<usize>,
    /// Whether the mark is *active*: active marks highlight the region
    /// between point and mark (transient-mark-mode). C-g deactivates.
    mark_active: bool,
    /// True when the buffer content should be treated as read-only
    /// (used for *Help* and dired listings).
    read_only: bool,
    /// Major mode (language for highlighting + name shown in the modeline).
    mode: Mode,
    /// Local keymap installed by the major mode / buffer-local bindings.
    local_keymap: Option<Keymap>,
    /// Names of enabled minor modes, in enable order (last = most recent).
    enabled_minor: Vec<String>,
}

/// A cloned buffer shares its text (used to save/restore the minibuffer
/// state across a nested command).
impl Clone for Buffer {
    fn clone(&self) -> Self {
        Buffer {
            id: self.id,
            name: self.name.clone(),
            path: self.path.clone(),
            rope: self.rope.clone(),
            point: self.point,
            goal_column: self.goal_column,
            modified: self.modified,
            mark: self.mark,
            mark_active: self.mark_active,
            read_only: self.read_only,
            mode: self.mode.clone(),
            local_keymap: self.local_keymap.clone(),
            enabled_minor: self.enabled_minor.clone(),
        }
    }
}

impl Buffer {
    pub fn new(name: impl Into<String>) -> Self {
        Buffer {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: name.into(),
            path: None,
            rope: Rope::new(),
            point: 0,
            goal_column: None,
            modified: false,
            mark: None,
            mark_active: false,
            read_only: false,
            mode: fundamental(),
            local_keymap: None,
            enabled_minor: Vec::new(),
        }
    }

    /// Build a buffer from any reader. Streaming, so peak memory stays flat
    /// even for very large inputs. Starts in fundamental mode; the Lua layer
    /// picks the mode from the file name.
    pub fn from_reader(name: impl Into<String>, reader: impl Read) -> std::io::Result<Self> {
        let rope = Rope::from_reader(BufReader::new(reader))?;
        Ok(Buffer {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: name.into(),
            path: None,
            rope,
            point: 0,
            goal_column: None,
            modified: false,
            mark: None,
            mark_active: false,
            read_only: false,
            mode: fundamental(),
            local_keymap: None,
            enabled_minor: Vec::new(),
        })
    }

    pub fn load_file(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let mut buf = Self::from_reader(name, file)?;
        buf.path = Some(path.to_path_buf());
        Ok(buf)
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = self.path.as_ref().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "buffer has no file name")
        })?;
        let mut file = File::create(path)?;
        for chunk in self.rope.chunks() {
            file.write_all(chunk.as_bytes())?;
        }
        Ok(())
    }

    /// Re-read the buffer's file from disk (auto-revert / revert-buffer):
    /// replace the text, clamp point and mark, clear the modified flag,
    pub fn reload_from_disk(&mut self) -> std::io::Result<()> {
        let path = self.path.clone().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "buffer has no file name")
        })?;
        let file = File::open(&path)?;
        let rope = Rope::from_reader(BufReader::new(file))?;
        self.rope = rope;
        self.point = self.point.min(self.rope.len_chars());
        if let Some(m) = self.mark.as_mut() {
            *m = (*m).min(self.rope.len_chars());
        }
        self.modified = false;
        Ok(())
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn point(&self) -> usize {
        self.point
    }

    pub fn set_point(&mut self, p: usize) {
        self.point = p.min(self.rope.len_chars());
    }

    pub fn mark(&self) -> Option<usize> {
        self.mark
    }

    /// Set the mark position and activate it (Emacs `set-mark-command`).
    pub fn set_mark(&mut self, m: Option<usize>) {
        self.mark = m.map(|m| m.min(self.rope.len_chars()));
        self.mark_active = m.is_some();
    }

    pub fn mark_active(&self) -> bool {
        self.mark_active
    }

    /// Deactivate the mark: the region stops being highlighted, but the
    /// mark position is kept (C-x C-x can reactivate it).
    pub fn deactivate_mark(&mut self) {
        self.mark_active = false;
    }

    pub fn activate_mark(&mut self) {
        if self.mark.is_some() {
            self.mark_active = true;
        }
    }

    /// Ordered (start, end) of the active region between mark and point,
    /// if the mark is set, active, and the region is non-empty.
    pub fn region(&self) -> Option<(usize, usize)> {
        if !self.mark_active {
            return None;
        }
        let m = self.mark?;
        if m == self.point {
            return None;
        }
        Some((m.min(self.point), m.max(self.point)))
    }

    pub fn modified(&self) -> bool {
        self.modified
    }

    pub fn set_modified(&mut self, m: bool) {
        self.modified = m;
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    pub fn set_read_only(&mut self, ro: bool) {
        self.read_only = ro;
    }

    pub fn mode(&self) -> &Mode {
        &self.mode
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    pub fn set_local_keymap(&mut self, keymap: Option<Keymap>) {
        self.local_keymap = keymap;
    }

    pub fn local_keymap(&self) -> Option<&Keymap> {
        self.local_keymap.as_ref()
    }

    pub fn local_keymap_mut(&mut self) -> &mut Keymap {
        self.local_keymap.get_or_insert_with(Keymap::new)
    }

    pub fn enabled_minor(&self) -> &[String] {
        &self.enabled_minor
    }

    pub fn minor_mode_enabled(&self, name: &str) -> bool {
        self.enabled_minor.iter().any(|m| m == name)
    }

    pub fn enable_minor_mode(&mut self, name: &str) {
        if !self.minor_mode_enabled(name) {
            self.enabled_minor.push(name.to_string());
        }
    }

    pub fn disable_minor_mode(&mut self, name: &str) {
        self.enabled_minor.retain(|m| m != name);
    }

    /// 0-based line index containing `point`.
    pub fn line_of_point(&self) -> usize {
        self.rope.char_to_line(self.point)
    }

    /// Column of point within its line, in chars, 0-based.
    pub fn column(&self) -> usize {
        let line_start = self.rope.line_to_char(self.line_of_point());
        self.point - line_start
    }

    /// The text of the given line without the trailing `\r`/`\n`.
    pub fn line(&self, idx: usize) -> ropey::RopeSlice<'_> {
        self.rope.line(idx.min(self.rope.len_lines() - 1))
    }

    /// Char length of the line, excluding trailing `\r`/`\n` (`\r\n` counts as
    /// one invisible unit).
    pub fn line_len_chars(&self, idx: usize) -> usize {
        let s = self.line(idx);
        let mut len = s.len_chars();
        if len >= 2 && s.char(len - 1) == '\n' && s.char(len - 2) == '\r' {
            len -= 2;
        } else if len > 0 && s.char(len - 1) == '\n' {
            len -= 1;
        }
        len
    }

    // --- motion ------------------------------------------------------------

    /// Offset right of `idx` by one *visible* char: a CRLF pair counts as one.
    pub fn step_right(&self, idx: usize) -> usize {
        if idx >= self.rope.len_chars() {
            return idx;
        }
        if self.rope.char(idx) == '\r'
            && idx + 1 < self.rope.len_chars()
            && self.rope.char(idx + 1) == '\n'
        {
            idx + 2
        } else {
            idx + 1
        }
    }

    /// Offset left of `idx` by one visible char. A CRLF pair before point
    /// counts as a single unit (Emacs treats `\r\n` as one newline).
    pub fn step_left(&self, idx: usize) -> usize {
        if idx == 0 {
            return 0;
        }
        if self.rope.char(idx - 1) == '\n' && idx >= 2 && self.rope.char(idx - 2) == '\r' {
            idx - 2
        } else {
            idx - 1
        }
    }

    pub fn move_char(&mut self, dir: Direction) {
        self.point = match dir {
            Direction::Forward => self.step_right(self.point),
            Direction::Backward => self.step_left(self.point),
        };
        self.goal_column = None;
    }

    pub fn move_to_line_start(&mut self) {
        self.point = self.rope.line_to_char(self.line_of_point());
        self.goal_column = None;
    }

    pub fn move_to_line_end(&mut self) {
        let line = self.line_of_point();
        let start = self.rope.line_to_char(line);
        self.point = start + self.line_len_chars(line);
        self.goal_column = None;
    }

    pub fn move_to_buffer_start(&mut self) {
        self.point = 0;
        self.goal_column = None;
    }

    pub fn move_to_buffer_end(&mut self) {
        self.point = self.rope.len_chars();
        self.goal_column = None;
    }

    /// Move to the given line, preserving the goal column (clamped to the
    /// line's length).
    pub fn move_to_line(&mut self, line: usize) {
        let goal = self.goal_column.unwrap_or_else(|| self.column());
        let line = line.min(self.rope.len_lines() - 1);
        let start = self.rope.line_to_char(line);
        self.point = start + goal.min(self.line_len_chars(line));
        self.goal_column = Some(goal);
    }

    pub fn move_line(&mut self, dir: Direction) {
        let line = self.line_of_point();
        let target = match dir {
            Direction::Forward => (line + 1).min(self.rope.len_lines() - 1),
            Direction::Backward => line.saturating_sub(1),
        };
        self.move_to_line(target);
    }

    /// Emacs `forward-word` semantics: from inside a word move to its end;
    /// otherwise skip non-word chars, then move to the end of the next word.
    pub fn move_word(&mut self, dir: Direction) {
        let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_';
        match dir {
            Direction::Forward => {
                let mut i = self.point;
                let len = self.rope.len_chars();
                while i < len && is_word(self.rope.char(i)) {
                    i = self.step_right(i);
                }
                if i == self.point {
                    while i < len && !is_word(self.rope.char(i)) {
                        i = self.step_right(i);
                    }
                    while i < len && is_word(self.rope.char(i)) {
                        i = self.step_right(i);
                    }
                }
                self.point = i;
            }
            Direction::Backward => {
                let mut i = self.point;
                while i > 0 && is_word(self.rope.char(self.step_left(i))) {
                    i = self.step_left(i);
                }
                if i == self.point {
                    while i > 0 && !is_word(self.rope.char(self.step_left(i))) {
                        i = self.step_left(i);
                    }
                    while i > 0 && is_word(self.rope.char(self.step_left(i))) {
                        i = self.step_left(i);
                    }
                }
                self.point = i;
            }
        }
        self.goal_column = None;
    }

    // --- editing -----------------------------------------------------------

    pub fn insert(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let len = text.chars().count();
        self.rope.insert(self.point, text);
        if let Some(m) = self.mark.as_mut() {
            if *m >= self.point {
                *m += len;
            }
        }
        self.point += len;
        self.goal_column = None;
        self.modified = true;
    }

    /// Insert `text` at `pos` (not necessarily point); point is unchanged.
    pub fn insert_at(&mut self, pos: usize, text: &str) {
        let pos = pos.min(self.rope.len_chars());
        let len = text.chars().count();
        if len == 0 {
            return;
        }
        self.rope.insert(pos, text);
        if let Some(m) = self.mark.as_mut() {
            if *m >= pos {
                *m += len;
            }
        }
        if self.point > pos {
            self.point += len;
        }
        self.modified = true;
    }

    pub fn insert_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.insert(c.encode_utf8(&mut buf));
    }

    /// Delete `start..end` and return the removed text. Adjusts point and
    /// mark; does not record undo information.
    pub fn delete_range(&mut self, start: usize, end: usize) -> String {
        if end <= start || start >= self.rope.len_chars() {
            return String::new();
        }
        let end = end.min(self.rope.len_chars());
        let text = self.rope.slice(start..end).to_string();
        self.rope.remove(start..end);
        if let Some(m) = self.mark.as_mut() {
            if *m >= start {
                *m = if *m <= end { start } else { *m - (end - start) };
            }
        }
        if self.point > start {
            self.point -= self.point.min(end) - start;
        }
        self.goal_column = None;
        self.modified = true;
        text
    }

    /// Delete one char before point (`delete-backward-char`).
    pub fn delete_backward(&mut self) -> bool {
        if self.point == 0 {
            return false;
        }
        let start = self.step_left(self.point);
        self.delete_range(start, self.point);
        true
    }

    /// Delete one char at point (`delete-char`).
    pub fn delete_forward(&mut self) -> bool {
        if self.point >= self.rope.len_chars() {
            return false;
        }
        let end = self.step_right(self.point);
        self.delete_range(self.point, end);
        true
    }

    /// Swap point and mark (`exchange-point-and-mark`).
    pub fn exchange_point_and_mark(&mut self) {
        if let Some(m) = self.mark {
            self.mark = Some(self.point);
            self.point = m;
        }
        self.goal_column = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(text: &str) -> Buffer {
        Buffer::from_reader("test", text.as_bytes()).unwrap()
    }

    #[test]
    fn motion_basic() {
        let mut b = buf("hello\nworld");
        b.move_to_buffer_end();
        assert_eq!(b.point(), 11);
        b.move_to_buffer_start();
        b.move_char(Direction::Forward);
        assert_eq!(b.point(), 1);
        b.move_to_line_end();
        assert_eq!(b.point(), 5);
        b.move_line(Direction::Forward);
        assert_eq!(b.point(), 11, "goal column 5 preserved");
    }

    #[test]
    fn goal_column_preserved() {
        let mut b = buf("abcd\nx\nefgh");
        b.move_char(Direction::Forward);
        b.move_char(Direction::Forward);
        b.move_line(Direction::Forward);
        assert_eq!(b.point(), 6, "clamped to end of short line");
        b.move_line(Direction::Forward);
        assert_eq!(b.point(), 9, "goal column 2 restored on next line");
    }

    #[test]
    fn crlf_motion() {
        let mut b = buf("ab\r\ncd\r\n");
        b.move_to_buffer_end();
        assert_eq!(b.point(), 8);
        b.move_to_buffer_start();
        b.move_char(Direction::Forward);
        b.move_char(Direction::Forward);
        assert_eq!(b.point(), 2, "point before CRLF pair");
        b.move_char(Direction::Forward);
        assert_eq!(b.point(), 4, "steps over CRLF as one unit");
        b.move_char(Direction::Backward);
        assert_eq!(b.point(), 2, "steps back over CRLF as one unit");
        b.move_to_line_end();
        assert_eq!(b.column(), 2);
    }

    #[test]
    fn word_motion() {
        let mut b = buf("foo bar_baz  qux");
        b.move_word(Direction::Forward);
        assert_eq!(b.point(), 3, "end of foo");
        b.move_word(Direction::Forward);
        assert_eq!(b.point(), 11, "end of bar_baz");
        b.move_word(Direction::Forward);
        assert_eq!(b.point(), 16, "end of qux");
        b.move_word(Direction::Backward);
        assert_eq!(b.point(), 13, "start of qux");
        b.move_word(Direction::Backward);
        assert_eq!(b.point(), 4, "start of bar_baz");
    }

    #[test]
    fn insert_delete() {
        let mut b = buf("abc");
        b.insert_char('X');
        assert_eq!(b.rope().to_string(), "Xabc");
        b.move_to_buffer_end();
        b.insert("yz");
        assert_eq!(b.rope().to_string(), "Xabcyz");
        b.delete_backward();
        b.delete_backward();
        assert_eq!(b.rope().to_string(), "Xabc");
        b.move_to_buffer_start();
        b.delete_forward();
        assert_eq!(b.rope().to_string(), "abc");
        assert!(b.modified());
    }

    #[test]
    fn insert_at_keeps_point_and_mark() {
        let mut b = buf("0123456789");
        b.set_mark(Some(4));
        b.set_point(6);
        b.insert_at(2, "xx");
        assert_eq!(b.rope().to_string(), "01xx23456789");
        assert_eq!(b.point(), 8, "point shifts with the insert");
        assert_eq!(b.mark(), Some(6), "mark shifts with the insert");
    }

    #[test]
    fn crlf_delete_pair() {
        let mut b = buf("a\r\nb");
        b.move_to_buffer_end();
        b.move_char(Direction::Backward);
        b.delete_forward();
        b.move_char(Direction::Backward);
        b.delete_forward();
        assert_eq!(b.rope().to_string(), "a");
        assert_eq!(b.point(), 1);
    }

    #[test]
    fn region_and_mark() {
        let mut b = buf("hello world");
        b.set_mark(Some(0));
        b.move_to_buffer_end();
        assert_eq!(b.region(), Some((0, 11)));
        b.exchange_point_and_mark();
        assert_eq!(b.point(), 0);
        assert_eq!(b.mark(), Some(11));
    }

    #[test]
    fn delete_range_adjusts_point_and_mark() {
        let mut b = buf("0123456789");
        b.set_mark(Some(2));
        b.set_point(8);
        assert_eq!(b.delete_range(2, 5), "234");
        assert_eq!(b.point(), 5);
        assert_eq!(b.mark(), Some(2));
        assert_eq!(b.rope().to_string(), "0156789");
    }
}
