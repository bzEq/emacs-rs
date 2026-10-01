//! Per-window scroll state: the first visible *visual* row (long lines
//! wrap to several visual rows) plus caches that make row computations
//! O(delta) for typical edits.

use std::cell::Cell;

use crate::buffer::Buffer;
use crate::wrap;

#[derive(Debug)]
pub struct View {
    /// First visible visual row (0-based across the whole buffer).
    pub top_row: usize,
    /// Cache: the line whose rows-before was last computed.
    cached_line: usize,
    cached_rows_before: usize,
    /// Render hint: the (row, line, segment, buffer length, width) of the
    /// last rendered first row, so the renderer can skip to it in O(delta)
    /// instead of walking every line from the top on each frame. The
    /// buffer length and width invalidate the hint when they change.
    pub hint: Cell<(usize, usize, usize, usize, usize)>,
}

impl Default for View {
    fn default() -> Self {
        View {
            top_row: 0,
            cached_line: usize::MAX,
            cached_rows_before: 0,
            hint: Cell::new((0, 0, 0, usize::MAX, 0)),
        }
    }
}

impl Clone for View {
    fn clone(&self) -> Self {
        View {
            top_row: self.top_row,
            cached_line: self.cached_line,
            cached_rows_before: self.cached_rows_before,
            hint: Cell::new((0, 0, 0, usize::MAX, 0)),
        }
    }
}

impl View {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset the scroll position and caches: called when the window
    /// switches to a different buffer (the scroll state belongs to the
    /// previous buffer).
    pub fn reset(&mut self) {
        self.top_row = 0;
        self.cached_line = usize::MAX;
        self.cached_rows_before = 0;
        self.hint = Cell::new((0, 0, 0, usize::MAX, 0));
    }

    /// Visual rows of the buffer lines before `line`, computed from the
    /// cached line so typical cursor motion is O(delta).
    fn rows_before(&mut self, buf: &Buffer, width: usize, line: usize) -> usize {
        if self.cached_line == usize::MAX {
            self.cached_rows_before = wrap::rows_between(buf, width, 0, line);
        } else if line >= self.cached_line {
            self.cached_rows_before += wrap::rows_between(buf, width, self.cached_line, line);
        } else {
            self.cached_rows_before -= wrap::rows_between(buf, width, line, self.cached_line);
        }
        self.cached_line = line;
        self.cached_rows_before
    }

    /// Keep the cursor's visual row inside the visible window.
    pub fn scroll_to_cursor(&mut self, buf: &Buffer, width: usize, height: usize) {
        let height = height.max(1);
        let line = buf.line_of_point();
        let col = buf.column();
        let rows_before = self.rows_before(buf, width, line);
        let row = rows_before + wrap::row_in_line(buf.line(line), width, col);
        if row < self.top_row {
            self.top_row = row;
        } else if row >= self.top_row + height {
            self.top_row = row + 1 - height;
        }
    }

    /// `scroll-up-command` (C-v): keep the cursor on its screen row, show
    /// the next page of visual rows.
    pub fn page_down(&mut self, buf: &mut Buffer, width: usize, height: usize) {
        let rows = height.saturating_sub(2).max(1);
        let cursor_row = wrap::row_of_point(buf, width, buf.point()).saturating_sub(self.top_row);
        self.top_row = self.top_row.saturating_add(rows);
        let target = self.top_row + cursor_row.min(height.saturating_sub(1));
        buf.set_point(wrap::pos_at_row(buf, width, target));
        self.cached_line = usize::MAX;
    }

    /// `scroll-down-command` (M-v).
    pub fn page_up(&mut self, buf: &mut Buffer, width: usize, height: usize) {
        let rows = height.saturating_sub(2).max(1);
        let cursor_row = wrap::row_of_point(buf, width, buf.point()).saturating_sub(self.top_row);
        self.top_row = self.top_row.saturating_sub(rows);
        let target = self.top_row + cursor_row.min(height.saturating_sub(1));
        buf.set_point(wrap::pos_at_row(buf, width, target));
        self.cached_line = usize::MAX;
    }

    /// `recenter` (C-l): center the cursor's visual row in the window.
    pub fn recenter(&mut self, buf: &Buffer, width: usize, height: usize) {
        let row = wrap::row_of_point(buf, width, buf.point());
        self.top_row = row.saturating_sub(height / 2);
        self.cached_line = usize::MAX;
    }
}
