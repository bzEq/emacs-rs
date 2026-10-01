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
    /// Width the rows-before cache was computed at: wrapped row counts
    /// depend on the width, so a width change (resize, split) invalidates
    /// the cached values.
    cached_width: usize,
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
            cached_width: usize::MAX,
            hint: Cell::new((0, 0, 0, usize::MAX, 0)),
        }
    }
}

impl Clone for View {
    fn clone(&self) -> Self {
        // The rows-before cache is copied: it stays valid when the split
        // doesn't change the width (C-x 2), and `rows_before` invalidates
        // it via `cached_width` when it does (C-x 3).
        View {
            top_row: self.top_row,
            cached_line: self.cached_line,
            cached_rows_before: self.cached_rows_before,
            cached_width: self.cached_width,
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
        self.cached_width = usize::MAX;
        self.hint = Cell::new((0, 0, 0, usize::MAX, 0));
    }

    /// Visual rows of the buffer lines before `line`, computed from the
    /// cached line so typical cursor motion is O(delta). A cold cache is
    /// seeded from the render hint (which knows the visual row of a recent
    /// line); without a usable hint it falls back to a full walk from the
    /// buffer start.
    fn rows_before(&mut self, buf: &Buffer, width: usize, line: usize) -> usize {
        if self.cached_width != width {
            self.cached_width = width;
            self.cached_line = usize::MAX;
        }
        if self.cached_line == usize::MAX {
            let (hrow, hline, hseg, hlen, hwidth) = self.hint.get();
            if hlen == buf.len_chars() && hwidth == width && hline < buf.len_lines() {
                self.cached_rows_before = hrow.saturating_sub(hseg);
                self.cached_line = hline;
            } else {
                self.cached_rows_before = wrap::rows_between(buf, width, 0, line);
                self.cached_line = line;
                return self.cached_rows_before;
            }
        }
        if line >= self.cached_line {
            self.cached_rows_before += wrap::rows_between(buf, width, self.cached_line, line);
        } else {
            self.cached_rows_before -= wrap::rows_between(buf, width, line, self.cached_line);
        }
        self.cached_line = line;
        self.cached_rows_before
    }

    /// The visual row of the buffer's point.
    fn point_row(&mut self, buf: &Buffer, width: usize) -> usize {
        let line = buf.line_of_point();
        let col = buf.column();
        self.rows_before(buf, width, line) + wrap::row_in_line(buf.line(line), width, col)
    }

    /// The char offset at the start of visual row `target`, seeded from
    /// the rows-before cache so page moves are O(delta) from the point.
    fn pos_at_row(&mut self, buf: &Buffer, width: usize, target: usize) -> usize {
        if self.cached_line == usize::MAX {
            return wrap::pos_at_row(buf, width, target);
        }
        let mut w = wrap::RowWalker::from_hint(
            buf,
            width,
            self.cached_rows_before,
            self.cached_line,
            0,
            target,
        );
        match w.next_row() {
            Some((l, _seg, s, _e)) => buf.rope().line_to_char(l) + s,
            None => buf.len_chars(),
        }
    }

    /// Keep the cursor's visual row inside the visible window.
    pub fn scroll_to_cursor(&mut self, buf: &Buffer, width: usize, height: usize) {
        self.scroll_to(buf, width, height, buf.point());
    }

    /// Keep the visual row of `point` (a char offset, not necessarily the
    /// buffer's point) inside the visible window.
    pub fn scroll_to(&mut self, buf: &Buffer, width: usize, height: usize, point: usize) {
        let height = height.max(1);
        let point = point.min(buf.len_chars());
        let line = buf.rope().char_to_line(point);
        let line_start = buf.rope().line_to_char(line);
        let col = point - line_start;
        let row =
            self.rows_before(buf, width, line) + wrap::row_in_line(buf.line(line), width, col);
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
        let cursor_row = self.point_row(buf, width).saturating_sub(self.top_row);
        self.top_row = self.top_row.saturating_add(rows);
        let target = self.top_row + cursor_row.min(height.saturating_sub(1));
        buf.set_point(self.pos_at_row(buf, width, target));
    }

    /// `scroll-down-command` (M-v).
    pub fn page_up(&mut self, buf: &mut Buffer, width: usize, height: usize) {
        let rows = height.saturating_sub(2).max(1);
        let cursor_row = self.point_row(buf, width).saturating_sub(self.top_row);
        self.top_row = self.top_row.saturating_sub(rows);
        let target = self.top_row + cursor_row.min(height.saturating_sub(1));
        buf.set_point(self.pos_at_row(buf, width, target));
    }

    /// `recenter` (C-l): center the cursor's visual row in the window.
    pub fn recenter(&mut self, buf: &Buffer, width: usize, height: usize) {
        let row = self.point_row(buf, width);
        self.top_row = row.saturating_sub(height / 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_view(content: &str) -> (Buffer, View) {
        let mut b = Buffer::new("test");
        b.insert(content);
        (b, View::new())
    }

    #[test]
    fn scroll_to_keeps_point_visible() {
        let mut content = String::new();
        for i in 0..100 {
            content.push_str(&format!("line {i} some padding here\n"));
        }
        let (buf, mut v) = lines_view(&content);
        let mut b = buf;
        b.set_point(80);
        v.scroll_to_cursor(&b, 80, 10);
        // the point's visual row is 2 (one row per line at width 80)
        assert!(
            v.top_row <= 2 && 2 < v.top_row + 10,
            "point row 2 inside window"
        );
    }

    #[test]
    fn width_change_invalidates_rows_cache() {
        // rows-before depends on the wrap width: after the width changes,
        // the cached value must be recomputed, not reused.
        let mut content = String::new();
        for _ in 0..50 {
            content.push_str("aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj kkkk\n");
        }
        let (buf, mut v) = lines_view(&content);
        let wide = v.rows_before(&buf, 80, 50);
        let narrow = v.rows_before(&buf, 20, 50);
        assert!(narrow > wide, "narrow width wraps into more rows");
        // and back: cached at 20, ask at 80 -> must not reuse the 20-cache
        assert_eq!(v.rows_before(&buf, 80, 50), wide);
    }

    #[test]
    fn clone_preserves_cache_for_same_width() {
        let mut content = String::new();
        for _ in 0..50 {
            content.push_str("some words wrap around at the width boundary yes\n");
        }
        let (buf, mut v) = lines_view(&content);
        let rows = v.rows_before(&buf, 40, 25);
        let mut cloned = v.clone();
        // the clone's cache stays valid at the same width: no full walk
        assert_eq!(
            cloned.rows_before(&buf, 40, 26),
            rows + wrap::row_count(buf.line(25), 40)
        );
    }
}
