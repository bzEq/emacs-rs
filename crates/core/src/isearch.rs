//! Incremental search (isearch): C-s / C-r.

use anyhow::Result;
use ropey::Rope;

use crate::editor::Editor;
use crate::key::{Key, KeyCode, Modifiers};

#[derive(Debug, Clone)]
pub struct ISearch {
    pub query: String,
    pub forward: bool,
    /// Point when the search started; C-g returns here.
    pub start: usize,
    /// (start, end) of the current match, if any.
    pub matched: Option<(usize, usize)>,
    pub failed: bool,
    pub wrapped: bool,
}

impl ISearch {
    pub fn new(forward: bool, start: usize) -> Self {
        ISearch {
            query: String::new(),
            forward,
            start,
            matched: None,
            failed: false,
            wrapped: false,
        }
    }
}

fn eq_ci(a: char, b: &str) -> bool {
    a.to_lowercase().eq(b.chars())
}

/// Find the first match of `query` (lowercased) at or after `from`,
/// case-insensitively. Returns (start, end) in char offsets.
pub fn find_forward(rope: &Rope, query: &[String], from: usize) -> Option<(usize, usize)> {
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
            return Some((i, i + query.len()));
        }
        i += 1;
        if i >= len {
            return None;
        }
    }
}

/// Find the last match of `query` strictly before `from`.
pub fn find_backward(rope: &Rope, query: &[String], from: usize) -> Option<(usize, usize)> {
    if query.is_empty() {
        return None;
    }
    let mut best = None;
    let mut i = 0;
    loop {
        let Some((s, e)) = find_forward(rope, query, i) else {
            break;
        };
        if s >= from {
            break;
        }
        best = Some((s, e));
        i = s + 1;
    }
    best
}

fn lower_query(q: &str) -> Vec<String> {
    q.chars().map(|c| c.to_lowercase().collect()).collect()
}

/// Run one search step from the current point, updating the isearch state.
/// `restart` means the query changed (search again from point, no wrap);
/// otherwise advance past the current match (C-s / C-r, wrapping allowed).
fn step(ed: &mut Editor, restart: bool) {
    let is = ed.take_isearch().expect("isearch active");
    let q = lower_query(&is.query);
    let rope = ed.buf().rope().clone();
    let point = ed.buf().point();
    let len = rope.len_chars();

    let from = if restart {
        point
    } else if is.forward {
        is.matched.map(|(s, _)| s + 1).unwrap_or(point + 1).min(len)
    } else {
        is.matched.map(|(s, _)| s).unwrap_or(point)
    };

    let found: Option<((usize, usize), bool)> = if is.forward {
        match find_forward(&rope, &q, from) {
            Some(m) => Some((m, false)),
            None if !restart && from > 0 => find_forward(&rope, &q, 0).map(|m| (m, true)),
            None => None,
        }
    } else {
        match find_backward(&rope, &q, from) {
            Some(m) => Some((m, false)),
            None if !restart && from < len => find_backward(&rope, &q, len).map(|m| (m, true)),
            None => None,
        }
    };

    let mut is = is;
    match found {
        Some(((s, e), wrapped)) => {
            is.matched = Some((s, e));
            is.failed = false;
            is.wrapped = wrapped;
            ed.buf_mut().set_point(s);
        }
        None => {
            is.matched = None;
            is.failed = !is.query.is_empty();
            is.wrapped = false;
        }
    }
    ed.set_isearch(Some(is));
}

fn prompt(ed: &Editor) -> String {
    let is = ed.isearch().expect("isearch active");
    let dir = if is.forward { "" } else { " backward" };
    let status = if is.failed { "Failing " } else { "" };
    let wrap = if is.wrapped { " (wrapped)" } else { "" };
    format!("{status}I-search{dir}: {}{wrap}", is.query)
}

/// Result of handling a key during isearch.
pub enum ISearchResult {
    /// The key was consumed; search continues.
    Consumed,
    /// Search is over; optionally replay the key in the normal keymap
    /// (Emacs runs non-search keys after leaving isearch).
    Exit { replay: Option<Key> },
}

/// Handle one key while isearch is active.
pub fn handle_key(ed: &mut Editor, key: &Key) -> Result<ISearchResult> {
    use ISearchResult::*;
    let m = key.mods;
    let ctrl = m.contains(Modifiers::CONTROL);
    let alt = m.contains(Modifiers::ALT);

    match key.code {
        KeyCode::Char(c) if ctrl && c == 'g' => {
            // abort: return to start
            let start = ed.isearch().map(|i| i.start).unwrap_or(0);
            ed.buf_mut().set_point(start);
            ed.set_isearch(None);
            ed.message("Quit");
            return Ok(Exit { replay: None });
        }
        KeyCode::Char(c) if ctrl && c == 's' => {
            // C-s: next match, or switch from backward to forward search.
            let forward = ed.isearch().map(|i| i.forward).unwrap_or(true);
            if !forward {
                let mut is = ed.take_isearch().expect("isearch active");
                is.forward = true;
                is.matched = None;
                ed.set_isearch(Some(is));
                step(ed, true);
            } else {
                step(ed, false);
            }
            ed.message(prompt(ed));
            return Ok(Consumed);
        }
        KeyCode::Char(c) if ctrl && c == 'r' => {
            // C-r: previous match, or switch from forward to backward search.
            let forward = ed.isearch().map(|i| i.forward).unwrap_or(true);
            if forward {
                let mut is = ed.take_isearch().expect("isearch active");
                is.forward = false;
                is.matched = None;
                ed.set_isearch(Some(is));
                step(ed, true);
            } else {
                step(ed, false);
            }
            ed.message(prompt(ed));
            return Ok(Consumed);
        }
        KeyCode::Char(c) if ctrl && c == 'w' => {
            // pull the word at point into the query
            let mut is = ed.take_isearch().expect("isearch active");
            let buf = ed.buf();
            let mut p = buf.point();
            let rope = buf.rope();
            while p < rope.len_chars() {
                let ch = rope.char(p);
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    is.query.push(ch);
                    p += 1;
                } else {
                    break;
                }
            }
            ed.set_isearch(Some(is));
            step(ed, true);
            ed.message(prompt(ed));
            return Ok(Consumed);
        }
        KeyCode::Backspace => {
            let mut is = ed.take_isearch().expect("isearch active");
            is.query.pop();
            is.matched = None;
            if is.query.is_empty() {
                ed.buf_mut().set_point(is.start);
            }
            ed.set_isearch(Some(is));
            step(ed, true);
            ed.message(prompt(ed));
            return Ok(Consumed);
        }
        KeyCode::Char(c) if !ctrl && !alt => {
            let mut is = ed.take_isearch().expect("isearch active");
            is.query.push(c);
            is.matched = None;
            ed.set_isearch(Some(is));
            step(ed, true);
            ed.message(prompt(ed));
            return Ok(Consumed);
        }
        KeyCode::Enter => {
            ed.set_isearch(None);
            ed.clear_echo();
            return Ok(Exit { replay: None });
        }
        KeyCode::Esc => {
            ed.set_isearch(None);
            ed.clear_echo();
            return Ok(Exit { replay: None });
        }
        _ => {
            // any other key leaves the search and runs normally
            ed.set_isearch(None);
            ed.clear_echo();
            return Ok(Exit { replay: Some(*key) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;
    use crate::key::{Key, KeyCode};

    fn ed_with(text: &str) -> Editor {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert(text);
        ed.buf_mut().move_to_buffer_start();
        ed
    }

    fn press(ed: &mut Editor, key: Key) -> ISearchResult {
        handle_key(ed, &key).unwrap()
    }

    #[test]
    fn forward_search_moves_point() {
        let mut ed = ed_with("hello world hello");
        ed.start_isearch(true);
        press(&mut ed, Key::plain('h'));
        assert_eq!(ed.buf().point(), 0);
        press(&mut ed, Key::plain('e'));
        assert_eq!(ed.buf().point(), 0, "extending the query keeps the match");
        press(&mut ed, Key::ctrl('s'));
        assert_eq!(ed.buf().point(), 12, "C-s jumps to the next match");
        press(&mut ed, Key::ctrl('s'));
        assert_eq!(ed.buf().point(), 0, "wraps around");
    }

    #[test]
    fn backspace_and_abort() {
        let mut ed = ed_with("ad x ad");
        ed.start_isearch(true);
        press(&mut ed, Key::plain('a'));
        press(&mut ed, Key::plain('d'));
        assert_eq!(ed.buf().point(), 0);
        press(&mut ed, Key::ctrl('s'));
        assert_eq!(ed.buf().point(), 5, "next match");
        press(&mut ed, Key::key(KeyCode::Backspace));
        assert_eq!(ed.buf().point(), 5, "re-searches from the current match");
        press(&mut ed, Key::ctrl('g'));
        assert!(!ed.isearch_active());
        assert_eq!(ed.buf().point(), 0, "C-g returns to the start");
    }

    #[test]
    fn backward_search() {
        let mut ed = ed_with("foo bar foo");
        ed.buf_mut().move_to_buffer_end();
        ed.start_isearch(false);
        press(&mut ed, Key::plain('f'));
        assert_eq!(ed.buf().point(), 8, "last match before point");
        press(&mut ed, Key::ctrl('r'));
        assert_eq!(ed.buf().point(), 0, "previous match");
    }

    #[test]
    fn enter_exits_at_match() {
        let mut ed = ed_with("one two three");
        ed.start_isearch(true);
        press(&mut ed, Key::plain('t'));
        press(&mut ed, Key::plain('w'));
        press(&mut ed, Key::key(KeyCode::Enter));
        assert!(!ed.isearch_active());
        assert_eq!(ed.buf().point(), 4);
    }

    #[test]
    fn failing_search() {
        let mut ed = ed_with("abc");
        ed.start_isearch(true);
        press(&mut ed, Key::plain('z'));
        assert!(ed.isearch().unwrap().failed);
        assert_eq!(ed.buf().point(), 0, "point unchanged on failure");
    }

    #[test]
    fn non_search_key_exits_and_replays() {
        let mut ed = ed_with("abc");
        ed.start_isearch(true);
        let r = press(&mut ed, Key::ctrl('a'));
        assert!(!ed.isearch_active());
        match r {
            ISearchResult::Exit { replay: Some(k) } => assert_eq!(k, Key::ctrl('a')),
            _ => panic!("expected replay"),
        }
    }
}
