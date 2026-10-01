//! Rope search primitives used by the Lua isearch implementation:
//! case-insensitive substring search over the buffer's rope.

use ropey::Rope;

/// Find the first case-insensitive match of `query` at or after `from`.
/// Returns the char offset of the match start.
pub fn find_forward(rope: &Rope, query: &str, from: usize) -> Option<usize> {
    let query: Vec<String> = query.chars().map(|c| c.to_lowercase().collect()).collect();
    let len = rope.len_chars();
    if query.is_empty() || from >= len {
        return None;
    }
    let mut i = from;
    loop {
        let mut j = 0;
        let mut ok = true;
        for ch in rope.slice(i..).chars().take(query.len()) {
            if !eq_ci(ch, &query[j]) {
                ok = false;
                break;
            }
            j += 1;
            if j == query.len() {
                break;
            }
        }
        if ok && j == query.len() {
            return Some(i);
        }
        i += 1;
        if i >= len {
            return None;
        }
    }
}

/// Find the last case-insensitive match of `query` strictly before `from`.
pub fn find_backward(rope: &Rope, query: &str, from: usize) -> Option<usize> {
    if query.is_empty() {
        return None;
    }
    let mut best = None;
    let mut i = 0;
    loop {
        let Some(s) = find_forward(rope, query, i) else {
            break;
        };
        if s >= from {
            break;
        }
        best = Some(s);
        i = s + 1;
    }
    best
}

fn eq_ci(a: char, b: &str) -> bool {
    a.to_lowercase().eq(b.chars())
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
        assert_eq!(find_forward(&r, "hello", 0), Some(0));
        assert_eq!(find_forward(&r, "hello", 1), Some(12));
        assert_eq!(find_forward(&r, "zzz", 0), None);
    }

    #[test]
    fn forward_is_case_insensitive() {
        let r = rope("Hello WORLD");
        assert_eq!(find_forward(&r, "hello", 0), Some(0));
        assert_eq!(find_forward(&r, "world", 0), Some(6));
    }

    #[test]
    fn backward_basic() {
        let r = rope("foo bar foo");
        assert_eq!(find_backward(&r, "foo", 11), Some(8));
        assert_eq!(find_backward(&r, "foo", 8), Some(0));
        assert_eq!(
            find_backward(&r, "foo", 1),
            Some(0),
            "match at 0 is before 1"
        );
        assert_eq!(
            find_backward(&r, "foo", 0),
            None,
            "no match strictly before 0"
        );
    }

    #[test]
    fn empty_query_finds_nothing() {
        let r = rope("abc");
        assert_eq!(find_forward(&r, "", 0), None);
        assert_eq!(find_backward(&r, "", 3), None);
    }
}
