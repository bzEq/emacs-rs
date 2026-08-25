//! Per-window scroll state. M1 keeps one window per buffer; M2 will split.

use crate::buffer::Buffer;

#[derive(Debug, Default, Clone, Copy)]
pub struct View {
    pub top_line: usize,
}

impl View {
    pub fn new() -> Self {
        Self::default()
    }

    fn clamp(&mut self, buf: &Buffer, height: usize) {
        let max_top = buf.len_lines().saturating_sub(height);
        self.top_line = self.top_line.min(max_top);
    }

    /// Keep the cursor line inside the visible window.
    pub fn scroll_to_cursor(&mut self, buf: &Buffer, height: usize) {
        let line = buf.line_of_point();
        if line < self.top_line {
            self.top_line = line;
        } else if line >= self.top_line + height {
            self.top_line = line + 1 - height;
        }
        self.clamp(buf, height);
    }

    /// `scroll-up-command` (C-v): keep cursor on its screen row, show next page.
    pub fn page_down(&mut self, buf: &mut Buffer, height: usize) {
        let rows = height.saturating_sub(2).max(1);
        let cursor_row = buf.line_of_point().saturating_sub(self.top_line);
        self.top_line = self.top_line.saturating_add(rows);
        self.clamp(buf, height);
        buf.move_to_line((self.top_line + cursor_row).min(buf.len_lines() - 1));
    }

    /// `scroll-down-command` (M-v).
    pub fn page_up(&mut self, buf: &mut Buffer, height: usize) {
        let rows = height.saturating_sub(2).max(1);
        let cursor_row = buf.line_of_point().saturating_sub(self.top_line);
        self.top_line = self.top_line.saturating_sub(rows);
        self.clamp(buf, height);
        buf.move_to_line((self.top_line + cursor_row).min(buf.len_lines() - 1));
    }

    /// `recenter` (C-l): center cursor line in the window.
    pub fn recenter(&mut self, buf: &Buffer, height: usize) {
        let line = buf.line_of_point();
        self.top_line = line.saturating_sub(height / 2);
        self.clamp(buf, height);
    }
}
