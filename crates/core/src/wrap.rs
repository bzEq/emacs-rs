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
///
/// Single pass over the chars: after a row breaks at a space, only the
/// short tail since that space is replayed as the next row's head (at most
/// `width` chars), so this is O(chars) and safe to call on huge lines.
pub fn wrap_ranges(content: RopeSlice<'_>, width: usize) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let n = content.len_chars();
    if n == 0 {
        return vec![(0, 0)];
    }
    let mut out = Vec::new();
    // (char index, char) of the current row, from its start.
    let mut row: Vec<(usize, char)> = Vec::new();
    let mut start = 0usize;
    let mut col = 0usize;
    // Char index one past the last whitespace of the current row.
    let mut last_space: Option<usize> = None;
    for (idx, c) in content.chars().enumerate() {
        let w = char_width(c, col);
        if col + w > width && !row.is_empty() {
            let end = match last_space {
                Some(ls) if ls > start => ls,
                _ => idx,
            };
            out.push((start, end));
            // The chars since `end` start the next row: replay them (they
            // are at most `width` long) to rebuild the column and space
            // state, then fall through to consume `c`.
            row.retain(|(i, _)| *i >= end);
            col = 0;
            last_space = None;
            for (i, cc) in &row {
                if cc.is_whitespace() {
                    last_space = Some(i + 1);
                }
                col += char_width(*cc, col);
            }
            start = end;
        }
        if c.is_whitespace() {
            last_space = Some(idx + 1);
        }
        col += w;
        row.push((idx, c));
    }
    out.push((start, n));
    out
}

/// Count the visual rows of `content` at `width` without allocating any
/// ranges (used when skipping lines during scroll walks, which must stay
/// cheap: large files walk hundreds of thousands of lines per frame).
/// This is `wrap_ranges(...).len()`, so it always agrees with wrapping.
pub fn row_count(content: RopeSlice<'_>, width: usize) -> usize {
    let width = width.max(1);
    let n = content.len_chars();
    if n == 0 {
        return 1;
    }
    // Fast path for contiguous ASCII lines without tabs: one byte per
    // column, no allocations, so counting millions of lines stays cheap.
    if let Some(s) = content.as_str() {
        if s.is_ascii() && !s.as_bytes().contains(&b'\t') {
            return ascii_row_count(s.as_bytes(), width);
        }
    }
    wrap_ranges(content, width).len()
}

/// Row count of an ASCII line (no tabs), one byte per column. Replays the
/// short tail after a space break, so this is O(bytes).
fn ascii_row_count(bytes: &[u8], width: usize) -> usize {
    let mut rows = 1usize;
    let mut col = 0usize;
    let mut row_start = 0usize;
    let mut last_space: Option<usize> = None; // one past the whitespace
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if col + 1 > width && i > row_start {
            rows += 1;
            match last_space {
                Some(ls) if ls > row_start => {
                    // re-measure the tail since the space as the new row
                    col = 0;
                    last_space = None;
                    for (k, &kb) in bytes.iter().enumerate().take(i).skip(ls) {
                        if kb.is_ascii_whitespace() {
                            last_space = Some(k + 1);
                        }
                        col += 1;
                    }
                    row_start = ls;
                }
                _ => {
                    col = 0;
                    last_space = None;
                    row_start = i;
                }
            }
        }
        if b.is_ascii_whitespace() {
            last_space = Some(i + 1);
        }
        col += 1;
        i += 1;
    }
    rows
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

/// Visual rows of the buffer lines in `from..to`. Counts newlines with
/// memchr; lines short enough to fit one visual row (the common case)
/// cost O(1), only long lines are measured character by character.
pub fn rows_between(buf: &Buffer, width: usize, from: usize, to: usize) -> usize {
    let width = width.max(1);
    let rope = buf.rope();
    let len_lines = rope.len_lines();
    let from = from.min(len_lines);
    let to = to.min(len_lines);
    if from >= to {
        return 0;
    }
    let byte_from = rope.line_to_byte(from);
    let byte_to = rope.line_to_byte(to);
    let mut rows = 0usize;
    let (chunks, mut chunk_byte, _, _) = rope.chunks_at_byte(byte_from);
    let mut line_start = byte_from;
    for chunk in chunks {
        if chunk_byte >= byte_to {
            break;
        }
        let bytes = chunk.as_bytes();
        let upto = (byte_to - chunk_byte).min(bytes.len());
        for rel in memchr::memchr_iter(b'\n', &bytes[..upto]) {
            let line_end = chunk_byte + rel;
            if line_end < line_start {
                continue; // newline before the starting line
            }
            rows += if line_end - line_start < width {
                1 // content shorter than a full row cannot wrap
            } else if line_start >= chunk_byte {
                // the whole line lives in this chunk: count its rows from
                // the bytes directly (no rope slicing per line)
                let lb = &bytes[line_start - chunk_byte..rel + 1];
                if lb.is_ascii() && !lb.contains(&b'\t') {
                    ascii_row_count(lb, width)
                } else {
                    row_count(rope.byte_slice(line_start..line_end + 1), width)
                }
            } else {
                row_count(rope.byte_slice(line_start..line_end + 1), width)
            };
            line_start = line_end + 1;
        }
        chunk_byte += chunk.len();
        if chunk_byte >= byte_to {
            break;
        }
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
    for (l, line) in buf.rope().lines().enumerate() {
        let rows = row_count(line, width);
        if r + rows > row {
            let (s, _) = wrap_ranges(line, width)[row - r];
            return buf.rope().line_to_char(l) + s;
        }
        r += rows;
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
        Self::from_hint(buf, width, 0, 0, 0, start_row)
    }

    /// Start at a known (line, segment) position at visual row `row` and
    /// skip forward to `target_row`. O(delta) when the hint is fresh; the
    /// caller validates the hint (buffer length unchanged, line in range,
    /// row <= target).
    pub fn from_hint(
        buf: &'a Buffer,
        width: usize,
        row: usize,
        line: usize,
        seg: usize,
        target_row: usize,
    ) -> Self {
        let width = width.max(1);
        // the hint's row is the first *rendered* row; normalize to the
        // row of the line's first segment
        let line_row = row.saturating_sub(seg);
        if target_row >= line_row {
            // forward: position at the line start and skip the difference
            let mut w = RowWalker {
                buf,
                width,
                remaining: target_row - line_row,
                line,
                ranges: Vec::new(),
                seg: 0,
            };
            w.load();
            return w;
        }
        // backward: walk up line by line until the target lands inside a
        // line (typically a short walk; scrolling backward far is rare)
        let mut needed = line_row - target_row;
        let mut cur = line;
        while needed > 0 && cur > 0 {
            cur -= 1;
            let rows = row_count(buf.line(cur), width);
            if rows >= needed {
                let ranges = wrap_ranges(buf.line(cur), width);
                let seg = (rows - needed).min(ranges.len().saturating_sub(1));
                return RowWalker {
                    buf,
                    width,
                    remaining: 0,
                    line: cur,
                    ranges,
                    seg,
                };
            }
            needed -= rows;
        }
        // above the buffer start: clamp to the very first row
        let ranges = wrap_ranges(buf.line(0), width);
        RowWalker {
            buf,
            width,
            remaining: 0,
            line: 0,
            ranges,
            seg: 0,
        }
    }

    /// Advance to the next row position and skip remaining rows.
    /// Iterative: with files of hundreds of thousands of lines, recursing
    /// per line would overflow the stack. Row counts (not ranges) are
    /// computed for fully skipped lines, so this stays cheap.
    fn load(&mut self) {
        loop {
            if self.line >= self.buf.len_lines() {
                self.ranges.clear();
                self.seg = 0;
                return;
            }
            if self.seg == 0 {
                // skip whole lines via a memchr newline scan: short lines
                // (the common case) are counted without measuring them
                let width = self.width;
                let byte_from = self
                    .buf
                    .rope()
                    .line_to_byte(self.line.min(self.buf.len_lines()));
                let (chunks, mut chunk_byte, _, _) = self.buf.rope().chunks_at_byte(byte_from);
                let mut line_start = byte_from;
                let mut lines = self.line;
                let mut skip = self.remaining;
                let mut landed = false;
                'outer: for chunk in chunks {
                    for rel in memchr::memchr_iter(b'\n', chunk.as_bytes()) {
                        let line_end = chunk_byte + rel;
                        if line_end < line_start {
                            continue; // newline before the starting line
                        }
                        let rows = if line_end - line_start < width {
                            1
                        } else if line_start >= chunk_byte {
                            let lb = &chunk.as_bytes()[line_start - chunk_byte..rel + 1];
                            if lb.is_ascii() && !lb.contains(&b'\t') {
                                ascii_row_count(lb, width)
                            } else {
                                row_count(
                                    self.buf.rope().byte_slice(line_start..line_end + 1),
                                    width,
                                )
                            }
                        } else {
                            row_count(self.buf.rope().byte_slice(line_start..line_end + 1), width)
                        };
                        if skip < rows {
                            self.ranges = wrap_ranges(self.buf.line(lines), width);
                            self.seg = skip;
                            self.remaining = 0;
                            self.line = lines;
                            landed = true;
                            break 'outer;
                        }
                        skip -= rows;
                        lines += 1;
                        line_start = line_end + 1;
                    }
                    chunk_byte += chunk.len();
                }
                if landed {
                    return;
                }
                // the scan ended without landing: either the remaining
                // rows ran out at the buffer end, or the last line has no
                // trailing newline
                self.line = lines;
                if self.line < self.buf.len_lines() {
                    self.ranges = wrap_ranges(self.buf.line(self.line), width);
                    self.seg = 0;
                    self.remaining = 0;
                } else {
                    self.ranges.clear();
                    self.seg = 0;
                }
                return;
            }
            // mid-line: consume the rest of the current line's ranges
            let left = self.ranges.len().saturating_sub(self.seg);
            if self.remaining < left {
                self.seg += self.remaining;
                self.remaining = 0;
                return;
            }
            self.remaining -= left;
            self.line += 1;
            self.seg = 0;
            self.ranges.clear();
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
            self.seg = 0;
            self.ranges.clear();
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

    #[test]
    fn row_count_matches_wrap_ranges() {
        // The row counter must agree with the range-based wrapping for
        // every width on a mix of short and long lines.
        let text = "short line\n".to_string()
            + "aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii jjjj kkkk llll mmmm\n"
            + "averyveryverylongwordthatbreaksmidword\n"
            + "one two three four five six seven eight nine ten eleven twelve\n";
        let rope = rope(&text);
        for width in 1..=40 {
            for line in rope.lines() {
                let ranges = wrap_ranges(line, width);
                assert_eq!(ranges.len(), row_count(line, width), "width {width}");
                let mut last = 0usize;
                for (s, e) in &ranges {
                    assert_eq!(*s, last, "ranges are contiguous");
                    last = *e;
                }
            }
        }
    }

    #[test]
    fn ascii_fast_path_agrees_with_general_path() {
        // The byte-level counter is exercised through `row_count` on a
        // contiguous ASCII rope; compare against the char-based ranges.
        let mut text = String::new();
        for i in 0..500 {
            text.push_str(&format!("some words and text {i} wrap me please now\n"));
        }
        let rope = rope(&text);
        for width in [5usize, 17, 39, 80] {
            for line in rope.lines().take(100) {
                assert_eq!(row_count(line, width), wrap_ranges(line, width).len());
            }
        }
    }

    #[test]
    fn rows_between_counts_long_lines() {
        let mut b = Buffer::new("test");
        // at width 5 "hello world\n" wraps to 4 rows (the trailing
        // newline counts as a char), "foo bar\n" and "baz qux\n" to 2
        b.insert("hello world\nfoo bar\nbaz qux\n");
        assert_eq!(rows_between(&b, 5, 0, 3), 8);
        assert_eq!(rows_between(&b, 5, 1, 3), 4);
        assert_eq!(rows_between(&b, 5, 0, 1), 4);
        // a width that fits everything: one row per line
        assert_eq!(rows_between(&b, 80, 0, 3), 3);
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;

    #[test]
    fn exact_width_content_wraps_to_two_rows() {
        // A line of exactly `width` content chars plus the trailing
        // newline occupies two visual rows: the newline overflows the
        // first row. The fast paths must agree with wrap_ranges here
        // (they used to count it as one row, desyncing the scroll and
        // the walkers by one row per such line).
        let mut b = Buffer::new("test");
        let content = "x".repeat(80);
        b.insert(&format!("{content}\n"));
        let rope = b.rope();
        assert_eq!(wrap_ranges(b.line(0), 80), vec![(0, 80), (80, 81)]);
        assert_eq!(row_count(b.line(0), 80), 2);
        assert_eq!(rows_between(&b, 80, 0, 1), 2);
        assert_eq!(rows_between(&b, 80, 0, rope.len_lines()), 2);
        // one char shorter fits on a single row
        b.insert("x");
        assert_eq!(
            row_count(b.line(0), 80),
            2,
            "81 chars + newline still wraps"
        );
        let mut c = Buffer::new("test");
        c.insert(&format!("{}\n", "y".repeat(79)));
        assert_eq!(wrap_ranges(c.line(0), 80), vec![(0, 80)]);
        assert_eq!(row_count(c.line(0), 80), 1);
        assert_eq!(rows_between(&c, 80, 0, c.rope().len_lines()), 1);
    }
}
