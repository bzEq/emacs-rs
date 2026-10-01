//! Visual line wrapping (word-wrap): shared by scrolling and rendering so
//! they agree on where lines break. Long lines wrap at word boundaries to
//! the window width; words longer than the width break mid-word.

use ropey::RopeSlice;

use crate::buffer::Buffer;

/// Display width of a tab character, in columns.
pub const TAB_WIDTH: usize = 8;

/// The visual column advance of a character (tabs expand to the next tab
/// stop).
pub fn char_width(c: char, col: usize) -> usize {
    if c == '\t' {
        TAB_WIDTH - col % TAB_WIDTH
    } else {
        1
    }
}

/// Visual width of the content (tab-expanded), in columns.
pub fn visual_width(content: RopeSlice<'_>) -> usize {
    let mut col = 0usize;
    for c in content.chars() {
        col += char_width(c, col);
    }
    col
}

/// Split a line's content into (start, end) char ranges, one per visual
/// row of `width` columns, breaking at the last whitespace when possible.
pub fn wrap_ranges(content: RopeSlice<'_>, width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let chars: Vec<char> = content.chars().collect();
    if chars.is_empty() {
        return vec![(0, 0)];
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let mut col = 0usize;
        let mut last_space: Option<usize> = None; // char index just past a whitespace
        let mut i = start;
        let mut end = chars.len();
        while i < chars.len() {
            let w = char_width(chars[i], col);
            if col + w > width && i > start {
                end = match last_space {
                    Some(ls) if ls > start => ls,
                    _ => i,
                };
                break;
            }
            if chars[i].is_whitespace() {
                last_space = Some(i + 1);
            }
            col += w;
            i += 1;
        }
        out.push((start, end));
        start = end;
    }
    out
}

/// The visual row (0-based, within the line) containing char column `col`.
pub fn row_in_line(content: RopeSlice<'_>, width: usize, col: usize) -> usize {
    for (i, (_, e)) in wrap_ranges(content, width).iter().enumerate() {
        if col <= *e {
            return i;
        }
    }
    0
}

/// Visual rows of the buffer lines in `from..to`.
pub fn rows_between(buf: &Buffer, width: usize, from: usize, to: usize) -> usize {
    let mut rows = 0usize;
    for l in from..to.min(buf.len_lines()) {
        rows += wrap_ranges(buf.line(l), width).len();
    }
    rows
}

/// The visual row of `point`, 0-based across the whole buffer.
pub fn row_of_point(buf: &Buffer, width: usize, point: usize) -> usize {
    let point = point.min(buf.len_chars());
    let line = buf.rope().char_to_line(point);
    let line_start = buf.rope().line_to_char(line);
    rows_between(buf, width, 0, line) + row_in_line(buf.line(line), width, point - line_start)
}

/// The char offset at the start of visual row `row` (clamped to the
/// buffer end).
pub fn pos_at_row(buf: &Buffer, width: usize, row: usize) -> usize {
    let mut r = 0usize;
    for l in 0..buf.len_lines() {
        let ranges = wrap_ranges(buf.line(l), width);
        if r + ranges.len() > row {
            let (s, _) = ranges[row - r];
            return buf.rope().line_to_char(l) + s;
        }
        r += ranges.len();
    }
    buf.len_chars()
}

/// Walks a buffer's visual rows, starting at a given row.
pub struct RowWalker<'a> {
    buf: &'a Buffer,
    width: usize,
    remaining: usize,
    line: usize,
    ranges: Vec<(usize, usize)>,
    seg: usize,
}

impl<'a> RowWalker<'a> {
    pub fn new(buf: &'a Buffer, width: usize, start_row: usize) -> Self {
        let mut w = RowWalker {
            buf,
            width: width.max(1),
            remaining: start_row,
            line: 0,
            ranges: Vec::new(),
            seg: 0,
        };
        w.load();
        w
    }

    /// Advance to the next line and skip remaining rows. Iterative: with
    /// files of hundreds of thousands of lines, recursing per line would
    /// overflow the stack.
    fn load(&mut self) {
        loop {
            if self.line >= self.buf.len_lines() {
                self.ranges.clear();
                self.seg = 0;
                return;
            }
            self.ranges = wrap_ranges(self.buf.line(self.line), self.width);
            self.seg = 0;
            let rows = self.ranges.len();
            if self.remaining < rows {
                self.seg = self.remaining;
                self.remaining = 0;
                return;
            }
            self.remaining -= rows;
            self.line += 1;
        }
    }

    /// The next visual row: (line index, segment index, char range within
    /// the line). Segment 0 is the first row of a line.
    pub fn next_row(&mut self) -> Option<(usize, usize, usize, usize)> {
        if self.line >= self.buf.len_lines() {
            return None;
        }
        let (s, e) = self.ranges[self.seg];
        let out = (self.line, self.seg, s, e);
        self.seg += 1;
        if self.seg >= self.ranges.len() {
            self.line += 1;
            self.load();
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> ropey::Rope {
        ropey::Rope::from_str(s)
    }

    #[test]
    fn wraps_at_width() {
        let r = rope("hello world foo");
        assert_eq!(wrap_ranges(r.slice(..), 8), vec![(0, 6), (6, 12), (12, 15)]);
    }

    #[test]
    fn breaks_at_last_space() {
        let r = rope("aaa bbbb cccc");
        assert_eq!(wrap_ranges(r.slice(..), 10), vec![(0, 9), (9, 13)]);
    }

    #[test]
    fn long_word_breaks_midword() {
        let r = rope("abcdefghij");
        assert_eq!(wrap_ranges(r.slice(..), 4), vec![(0, 4), (4, 8), (8, 10)]);
    }

    #[test]
    fn short_line_is_one_row() {
        let r = rope("hi");
        assert_eq!(wrap_ranges(r.slice(..), 80), vec![(0, 2)]);
    }

    #[test]
    fn empty_line_is_one_row() {
        let r = rope("");
        assert_eq!(wrap_ranges(r.slice(..), 80), vec![(0, 0)]);
    }

    #[test]
    fn tabs_expand_in_wrapping() {
        let r = rope("a\tb\tc");
        // a=1, tab to 8, b=9, tab to 16, c=17 -> width 10 breaks at the tab
        assert_eq!(wrap_ranges(r.slice(..), 10), vec![(0, 2), (2, 5)]);
    }

    #[test]
    fn row_of_point_counts_wrapped_rows() {
        let mut b = Buffer::new("test");
        b.insert("aaaa bbbb cccc\ndddd\n");
        assert_eq!(row_of_point(&b, 5, 0), 0);
        assert_eq!(row_of_point(&b, 5, 6), 1, "bbbb on row 1");
        assert_eq!(row_of_point(&b, 5, 15), 3, "dddd on row 3");
    }

    #[test]
    fn pos_at_row_maps_back() {
        let mut b = Buffer::new("test");
        b.insert("aaaa bbbb cccc\ndddd\n");
        assert_eq!(pos_at_row(&b, 5, 1), 5);
        assert_eq!(pos_at_row(&b, 5, 2), 10);
        assert_eq!(pos_at_row(&b, 5, 3), 15);
    }

    #[test]
    fn walker_skips_to_row() {
        let mut b = Buffer::new("test");
        b.insert("aaaa bbbb cccc\ndddd\n");
        let mut w = RowWalker::new(&b, 5, 2);
        let (line, seg, s, e) = w.next_row().unwrap();
        assert_eq!((line, seg, s, e), (0, 2, 10, 15));
        let (line, seg, s, e) = w.next_row().unwrap();
        assert_eq!((line, seg, s, e), (1, 0, 0, 5));
    }
}
