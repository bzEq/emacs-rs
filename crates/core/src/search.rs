//! Rope search primitives used by the Lua isearch implementation:
//! case-insensitive substring search over the buffer's rope.

use ropey::Rope;

/// Find the first case-insensitive match of `query` at or after `from`.
/// Returns the char offset of the match start.
pub fn find_forward(rope: &Rope, query: &str, from: usize) -> Option<usize> {
    let query = lowercased(query);
    let len = rope.len_chars();
    if query.is_empty() || from >= len {
        return None;
    }
    let mut i = from;
    loop {
        if matches_at(rope, &query, i) {
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
    let query = lowercased(query);
    if query.is_empty() {
        return None;
    }
    let mut i = from.min(rope.len_chars());
    loop {
        if i == 0 {
            return None;
        }
        i -= 1;
        if matches_at(rope, &query, i) {
            return Some(i);
        }
    }
}

/// Lowercase the query once per search. Each element is the lowercase
/// expansion of one query char (usually a single char).
fn lowercased(query: &str) -> Vec<String> {
    query.chars().map(|c| c.to_lowercase().collect()).collect()
}

/// Whether `query` matches the rope at char offset `i` (case-insensitive).
fn matches_at(rope: &Rope, query: &[String], i: usize) -> bool {
    let mut j = 0;
    for ch in rope.slice(i..).chars().take(query.len()) {
        if !eq_ci(ch, &query[j]) {
            return false;
        }
        j += 1;
        if j == query.len() {
            break;
        }
    }
    j == query.len()
}

fn eq_ci(a: char, b: &str) -> bool {
    let mut bi = b.chars();
    for la in a.to_lowercase() {
        match bi.next() {
            Some(lb) if la == lb => {}
            _ => return false,
        }
    }
    bi.next().is_none()
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

    #[test]
    fn forward_multibyte_before_match() {
        let r = rope("中x");
        assert_eq!(find_forward(&r, "x", 0), Some(1));
        assert_eq!(find_forward(&r, "中", 0), Some(0));
    }

    #[test]
    fn forward_multibyte_no_match() {
        let r = rope("中文文本");
        assert_eq!(find_forward(&r, "zzz", 0), None);
        assert_eq!(find_forward(&r, "文本", 0), Some(2));
    }

    #[test]
    fn backward_multibyte() {
        let r = rope("中abc");
        assert_eq!(find_backward(&r, "中", 10), Some(0));
        assert_eq!(find_backward(&r, "abc", 10), Some(1));
        assert_eq!(find_backward(&r, "中", 1), Some(0));
        assert_eq!(find_backward(&r, "中", 0), None);
    }

    #[test]
    fn backward_multibyte_after_match() {
        let r = rope("中a中");
        assert_eq!(find_backward(&r, "中", 2), Some(0));
        assert_eq!(find_backward(&r, "中", 3), Some(2));
        assert_eq!(find_backward(&r, "中", 1), Some(0));
    }

    #[test]
    fn emoji_query() {
        let r = rope("hi 😀 there");
        assert_eq!(find_forward(&r, "😀", 0), Some(3));
        assert_eq!(find_backward(&r, "😀", 10), Some(3));
    }

    #[test]
    fn match_at_boundary_steps() {
        let r = rope("😀😀x");
        assert_eq!(find_forward(&r, "x", 0), Some(2));
        assert_eq!(find_backward(&r, "😀", 3), Some(1));
    }
}
