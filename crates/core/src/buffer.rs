//! Rope-backed text buffer with Emacs-style cursor semantics.

use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ropey::Rope;

use crate::keymap::Keymap;
use crate::mode::{fundamental, mode_for_path, Mode};
use crate::syntax::Syntax;
use crate::undo::UndoLog;

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Backward,
}

impl Direction {
    pub fn reverse(self) -> Self {
        match self {
            Direction::Forward => Direction::Backward,
            Direction::Backward => Direction::Forward,
        }
    }
}

/// An Emacs-like text buffer backed by a `ropey::Rope`.
///
/// Point is stored as a char offset into the rope. All edits are O(log n).
/// Vertical motion remembers the goal column (Emacs MOVE_TO_VAR semantics).
/// CRLF files are treated as LF: cursor motion steps over `\r\n` as a unit
/// and the trailing `\r` is invisible to point.
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
    /// The mark, if set (C-SPC).
    mark: Option<usize>,
    undo: UndoLog,
    /// True when the buffer content should be treated as read-only
    /// (used for *Help*).
    read_only: bool,
    /// Major mode (language + indent behavior).
    mode: Mode,
    /// Local keymap installed by the major mode / buffer-local bindings.
    local_keymap: Option<Keymap>,
    /// Names of enabled minor modes, in enable order (last = most recent).
    enabled_minor: Vec<String>,
    /// Dired directory listing state, if this is a dired buffer.
    dired: Option<crate::dired::DiredState>,
    /// Parsed syntax tree for highlighting, if the mode has a language.
    syntax: Option<Syntax>,
    /// Set by edits while a language mode is active; triggers re-parse.
    syntax_dirty: bool,
    /// Last re-parse time, for edit throttling.
    syntax_last_parse: Option<std::time::Instant>,
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
            undo: UndoLog::default(),
            read_only: false,
            mode: fundamental(),
            local_keymap: None,
            enabled_minor: Vec::new(),
            dired: None,
            syntax: None,
            syntax_dirty: false,
            syntax_last_parse: None,
        }
    }

    /// Build a buffer from any reader. Streaming, so peak memory stays flat
    /// even for very large inputs. The mode is picked from `name` (file
    /// extension).
    pub fn from_reader(name: impl Into<String>, reader: impl Read) -> std::io::Result<Self> {
        let name = name.into();
        let mode = mode_for_path(&name);
        let has_lang = mode.lang.is_some();
        let rope = Rope::from_reader(BufReader::new(reader))?;
        Ok(Buffer {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name,
            path: None,
            rope,
            point: 0,
            goal_column: None,
            modified: false,
            mark: None,
            undo: UndoLog::default(),
            read_only: false,
            mode,
            local_keymap: None,
            enabled_minor: Vec::new(),
            dired: None,
            syntax: None,
            syntax_dirty: has_lang,
            syntax_last_parse: None,
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

    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
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

    pub fn set_mark(&mut self, m: Option<usize>) {
        self.mark = m.map(|m| m.min(self.rope.len_chars()));
    }

    /// Ordered (start, end) of the region between mark and point, if the mark
    /// is set and the region is non-empty.
    pub fn region(&self) -> Option<(usize, usize)> {
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

    pub fn dired(&self) -> Option<&crate::dired::DiredState> {
        self.dired.as_ref()
    }

    pub fn dired_mut(&mut self) -> Option<&mut crate::dired::DiredState> {
        self.dired.as_mut()
    }

    pub fn set_dired(&mut self, dired: Option<crate::dired::DiredState>) {
        self.dired = dired;
    }

    pub fn syntax(&self) -> Option<&Syntax> {
        self.syntax.as_ref()
    }

    pub fn set_syntax(&mut self, syntax: Option<Syntax>) {
        self.syntax = syntax;
    }

    pub fn syntax_dirty(&self) -> bool {
        self.syntax_dirty
    }

    pub fn set_syntax_dirty(&mut self, dirty: bool) {
        self.syntax_dirty = dirty;
    }

    pub fn syntax_last_parse(&self) -> Option<std::time::Instant> {
        self.syntax_last_parse
    }

    pub fn set_syntax_last_parse(&mut self, t: std::time::Instant) {
        self.syntax_last_parse = Some(t);
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
        self.undo.record_insert(self.point, len);
        if let Some(m) = self.mark.as_mut() {
            if *m >= self.point {
                *m += len;
            }
        }
        self.point += len;
        self.goal_column = None;
        self.modified = true;
        self.syntax_dirty |= self.mode.lang.is_some();
    }

    pub fn insert_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.insert(c.encode_utf8(&mut buf));
    }

    /// Delete `start..end` and return the removed text. Adjusts point and
    /// mark; records an undo entry.
    pub fn delete_range(&mut self, start: usize, end: usize) -> String {
        if end <= start || start >= self.rope.len_chars() {
            return String::new();
        }
        let end = end.min(self.rope.len_chars());
        let text = self.rope.slice(start..end).to_string();
        self.rope.remove(start..end);
        self.undo.record_delete(start, text.clone());
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
        self.syntax_dirty |= self.mode.lang.is_some();
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

    // --- undo --------------------------------------------------------------

    /// Add an undo boundary if the last entry isn't one.
    pub fn undo_boundary(&mut self) {
        self.undo.boundary();
    }

    /// Undo the most recent group of changes. Returns false if there is
    /// nothing to undo.
    pub fn undo(&mut self) -> bool {
        if self.undo.is_empty() {
            return false;
        }
        self.undo.pop_boundary();
        let mut did = false;
        loop {
            match self.undo.pop() {
                None => break,
                Some(crate::undo::UndoEntry::Boundary) => break,
                Some(crate::undo::UndoEntry::Insert { pos, len }) => {
                    let end = (pos + len).min(self.rope.len_chars());
                    self.rope.remove(pos..end);
                    self.point = pos;
                    did = true;
                }
                Some(crate::undo::UndoEntry::Delete { pos, text }) => {
                    self.rope.insert(pos, &text);
                    self.point = pos + text.chars().count();
                    did = true;
                }
            }
        }
        self.undo.boundary();
        self.modified = true;
        did
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

    #[test]
    fn undo_insert_and_delete() {
        let mut b = buf("ab");
        b.undo_boundary();
        b.move_to_buffer_end();
        b.insert("cd");
        assert_eq!(b.rope().to_string(), "abcd");
        b.undo_boundary();
        b.move_to_buffer_start();
        b.delete_forward();
        assert_eq!(b.rope().to_string(), "bcd");
        // undo the delete
        assert!(b.undo());
        assert_eq!(b.rope().to_string(), "abcd");
        assert_eq!(b.point(), 1);
        // undo the insert
        assert!(b.undo());
        assert_eq!(b.rope().to_string(), "ab");
        assert_eq!(b.point(), 2);
    }

    #[test]
    fn undo_group_until_boundary() {
        let mut b = buf("");
        b.undo_boundary();
        b.insert("a");
        b.insert("b");
        b.insert("c");
        assert!(b.undo());
        assert_eq!(b.rope().to_string(), "", "one undo removes the whole group");
        assert!(!b.undo(), "nothing left to undo");
    }
}
