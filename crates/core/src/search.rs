//! Rope search primitives used by the Lua isearch implementation:
//! case-insensitive substring search over the buffer's rope.
//!
//! Performance notes: queries whose first character is ASCII (the common
//! case) are accelerated with memchr/memrchr over the rope's chunks, and
//! character comparisons are allocation-free on the ASCII path. Non-ASCII
//! first characters fall back to a plain char scan. `find_backward` is a
//! single reverse pass (not repeated forward scans).

use ropey::Rope;

/// A lowercased query character: ASCII fast path, full expansion otherwise.
enum QChar {
    Ascii(u8),
    Char(String),
}

fn lower_query(query: &str) -> Vec<QChar> {
    query
        .chars()
        .map(|c| {
            if c.is_ascii() {
                QChar::Ascii(c.to_ascii_lowercase() as u8)
            } else {
                QChar::Char(c.to_lowercase().collect())
            }
        })
        .collect()
}

/// Case-insensitive equality of a haystack char with a query char. The
/// ASCII path (both sides ASCII) allocates nothing.
fn eq_ci(c: char, q: &QChar) -> bool {
    match q {
        QChar::Ascii(lo) => c.is_ascii() && c.to_ascii_lowercase() as u8 == *lo,
        QChar::Char(s) => c.to_lowercase().eq(s.chars()),
    }
}

/// Does `query` match the rope at char offset `start`?
fn matches_at(rope: &Rope, start: usize, q: &[QChar]) -> bool {
    if start + q.len() > rope.len_chars() {
        return false;
    }
    for (j, qc) in q.iter().enumerate() {
        let Some(c) = rope.get_char(start + j) else {
            return false;
        };
        if !eq_ci(c, qc) {
            return false;
        }
    }
    true
}

/// Verify a candidate match at byte offset `pos`, comparing in-chunk bytes
/// while both the query and the haystack are ASCII (the common case) and
/// falling back to rope char comparisons otherwise. This is what makes
/// searches over huge files fast: a query like "regclass" produces
/// millions of first-char candidates, and most must be rejected as
/// cheaply as possible.
fn verify_at(rope: &Rope, chunk: &str, chunk_byte: usize, rel: usize, q: &[QChar]) -> bool {
    let pos = chunk_byte + rel;
    let bytes = chunk.as_bytes();
    let mut j = 0usize;
    let mut off = rel;
    while j < q.len() {
        let QChar::Ascii(lo) = q[j] else {
            break;
        };
        if off >= bytes.len() || !bytes[off].is_ascii() {
            break;
        }
        if bytes[off].to_ascii_lowercase() != lo {
            return false;
        }
        j += 1;
        off += 1;
    }
    if j == q.len() {
        return true;
    }
    // the rest crosses a non-ASCII char or a chunk boundary: compare via
    // the rope, in char offsets
    let char_pos = rope.byte_to_char(pos);
    for (k, qc) in q[j..].iter().enumerate() {
        let Some(c) = rope.get_char(char_pos + j + k) else {
            return false;
        };
        if !eq_ci(c, qc) {
            return false;
        }
    }
    true
}

/// Find the first case-insensitive match of `query` at or after `from`.
/// Returns the (start, end) char offsets of the match.
pub fn find_forward(rope: &Rope, query: &str, from: usize) -> Option<(usize, usize)> {
    let q = lower_query(query);
    let len = rope.len_chars();
    if q.is_empty() || from >= len {
        return None;
    }
    let QChar::Ascii(first) = q[0] else {
        // non-ASCII first char: plain char scan
        let mut i = from;
        while i + q.len() <= len {
            if matches_at(rope, i, &q) {
                return Some((i, i + q.len()));
            }
            i += 1;
        }
        return None;
    };
    let first_up = first.to_ascii_uppercase();
    let byte_from = rope.char_to_byte(from);
    let (chunks, first_chunk_byte, _, _) = rope.chunks_at_byte(byte_from);
    let mut chunk_byte = first_chunk_byte;
    for chunk in chunks {
        let search = byte_from.saturating_sub(chunk_byte);
        let bytes = chunk.as_bytes();
        for rel in memchr::memchr2_iter(first, first_up, &bytes[search..]) {
            let rel = search + rel;
            if verify_at(rope, chunk, chunk_byte, rel, &q) {
                let char_idx = rope.byte_to_char(chunk_byte + rel);
                return Some((char_idx, char_idx + q.len()));
            }
        }
        chunk_byte += chunk.len();
    }
    None
}

/// Find the last case-insensitive match of `query` strictly before `from`.
/// Returns the (start, end) char offsets of the match.
pub fn find_backward(rope: &Rope, query: &str, from: usize) -> Option<(usize, usize)> {
    let q = lower_query(query);
    let len = rope.len_chars();
    if q.is_empty() || from == 0 {
        return None;
    }
    let QChar::Ascii(first) = q[0] else {
        // non-ASCII first char: plain scan of every char start before `from`
        let max_start = (from - 1).min(len.saturating_sub(q.len()));
        let mut i = max_start + 1;
        while i > 0 {
            i -= 1;
            if matches_at(rope, i, &q) {
                return Some((i, i + q.len()));
            }
        }
        return None;
    };
    let first_up = first.to_ascii_uppercase();
    // candidates must start at a byte strictly before char_to_byte(from)
    let byte_limit = rope.char_to_byte(from.min(len));
    // collect the chunk byte ranges up to the limit, then scan them in
    // reverse so the nearest match wins on first verification
    let mut chunks: Vec<(usize, &str)> = Vec::new();
    let mut byte = 0usize;
    for chunk in rope.chunks() {
        if byte >= byte_limit {
            break;
        }
        chunks.push((byte, chunk));
        byte += chunk.len();
    }
    for (chunk_byte, chunk) in chunks.iter().rev() {
        let upto = (byte_limit - chunk_byte).min(chunk.len());
        let slice = &chunk.as_bytes()[..upto];
        let mut pos = memchr::memrchr2(first, first_up, slice);
        while let Some(rel) = pos {
            if verify_at(rope, chunk, *chunk_byte, rel, &q) {
                let char_idx = rope.byte_to_char(chunk_byte + rel);
                return Some((char_idx, char_idx + q.len()));
            }
            if rel == 0 {
                break;
            }
            pos = memchr::memrchr2(first, first_up, &slice[..rel]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(s: &str) -> Rope {
        Rope::from_str(s)
    }

    #[test]
    fn forward_basic() {
        let r = rope("hello world hello");
        assert_eq!(find_forward(&r, "hello", 0), Some((0, 5)));
        assert_eq!(find_forward(&r, "hello", 1), Some((12, 17)));
        assert_eq!(find_forward(&r, "zzz", 0), None);
    }

    #[test]
    fn forward_is_case_insensitive() {
        let r = rope("Hello WORLD");
        assert_eq!(find_forward(&r, "hello", 0), Some((0, 5)));
        assert_eq!(find_forward(&r, "world", 0), Some((6, 11)));
        assert_eq!(
            find_forward(&r, "HELLO", 0),
            Some((0, 5)),
            "query case folded"
        );
    }

    #[test]
    fn forward_mixed_case_everywhere() {
        let r = rope("hElLo HeLLo");
        assert_eq!(find_forward(&r, "HELLO", 0), Some((0, 5)));
        assert_eq!(find_forward(&r, "hello", 1), Some((6, 11)));
    }

    #[test]
    fn forward_non_ascii_first_char() {
        let r = rope("中文 text 中文");
        assert_eq!(find_forward(&r, "中文", 0), Some((0, 2)));
        assert_eq!(find_forward(&r, "中文", 1), Some((8, 10)));
        assert_eq!(find_forward(&r, "中文", 10), None);
    }

    #[test]
    fn forward_crosses_chunk_boundaries() {
        // ropey chunks every ~512 bytes; make a match straddle a boundary
        let r = rope(&format!("{}hello", "a".repeat(511)));
        assert_eq!(find_forward(&r, "hello", 0), Some((511, 516)));
    }

    #[test]
    fn backward_basic() {
        let r = rope("foo bar foo");
        assert_eq!(find_backward(&r, "foo", 11), Some((8, 11)));
        assert_eq!(find_backward(&r, "foo", 8), Some((0, 3)));
        assert_eq!(
            find_backward(&r, "foo", 1),
            Some((0, 3)),
            "match at 0 is before 1"
        );
        assert_eq!(
            find_backward(&r, "foo", 0),
            None,
            "no match strictly before 0"
        );
    }

    #[test]
    fn backward_is_case_insensitive() {
        let r = rope("Foo BAR foo");
        assert_eq!(find_backward(&r, "FOO", 11), Some((8, 11)));
        assert_eq!(find_backward(&r, "bar", 8), Some((4, 7)));
    }

    #[test]
    fn backward_non_ascii_first_char() {
        let r = rope("中文 x 中文");
        assert_eq!(find_backward(&r, "中文", 8), Some((5, 7)));
        assert_eq!(find_backward(&r, "中文", 5), Some((0, 2)));
        assert_eq!(
            find_backward(&r, "中文", 1),
            Some((0, 2)),
            "match at 0 is before 1"
        );
    }

    #[test]
    fn empty_query_finds_nothing() {
        let r = rope("abc");
        assert_eq!(find_forward(&r, "", 0), None);
        assert_eq!(find_backward(&r, "", 3), None);
    }
}
