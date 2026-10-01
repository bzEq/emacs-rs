//! Incremental search, driven through the real Lua runtime with read-key
//! coroutine resumes (ported from the old Rust isearch tests).

mod common;

use common::LuaEd;
use emacs_core::key::{Key, KeyCode};
use emacs_core::script::PendingRequest;

fn start_search(le: &mut LuaEd, forward: bool) {
    let name = if forward {
        "isearch-forward"
    } else {
        "isearch-backward"
    };
    let outcome = le.command(name);
    assert!(le.ed.finish_command(outcome).is_none());
    assert!(
        matches!(le.ed.pending(), Some(PendingRequest::ReadKey)),
        "isearch waits for keys"
    );
}

#[test]
fn forward_search_moves_point() {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert("hello world hello");
    le.ed.buf_mut().move_to_buffer_start();
    start_search(&mut le, true);
    le.read_key(Key::plain('h'));
    assert_eq!(le.point(), 0);
    le.read_key(Key::plain('e'));
    assert_eq!(le.point(), 0, "extending the query keeps the match");
    le.read_key(Key::ctrl('s'));
    assert_eq!(le.point(), 12, "C-s jumps to the next match");
    le.read_key(Key::ctrl('s'));
    assert_eq!(le.point(), 0, "wraps around");
    le.read_key(Key::ctrl('g'));
    assert_eq!(le.point(), 0, "C-g returns to the start");
}

#[test]
fn backspace_and_abort() {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert("ad x ad");
    le.ed.buf_mut().move_to_buffer_start();
    start_search(&mut le, true);
    le.read_key(Key::plain('a'));
    le.read_key(Key::plain('d'));
    assert_eq!(le.point(), 0);
    le.read_key(Key::ctrl('s'));
    assert_eq!(le.point(), 5, "next match");
    le.read_key(Key::key(KeyCode::Backspace));
    assert_eq!(le.point(), 5, "re-searches from the current match");
    le.read_key(Key::ctrl('g'));
    assert_eq!(le.point(), 0, "C-g returns to the start");
}

#[test]
fn backward_search() {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert("foo bar foo");
    le.ed.buf_mut().move_to_buffer_end();
    start_search(&mut le, false);
    le.read_key(Key::plain('f'));
    assert_eq!(le.point(), 8, "last match before point");
    le.read_key(Key::ctrl('r'));
    assert_eq!(le.point(), 0, "previous match");
    le.read_key(Key::ctrl('g'));
}

#[test]
fn enter_exits_at_match() {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert("one two three");
    le.ed.buf_mut().move_to_buffer_start();
    start_search(&mut le, true);
    le.read_key(Key::plain('t'));
    le.read_key(Key::plain('w'));
    assert!(!le.read_key(Key::key(KeyCode::Enter)), "search finished");
    assert_eq!(le.point(), 4);
}

#[test]
fn failing_search() {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert("abc");
    le.ed.buf_mut().move_to_buffer_start();
    start_search(&mut le, true);
    le.read_key(Key::plain('z'));
    assert_eq!(le.point(), 0, "point unchanged on failure");
    le.read_key(Key::ctrl('g'));
    assert_eq!(le.ed.echo(), Some("Quit"));
}

#[test]
fn current_match_is_highlighted() {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert("hello world hello");
    le.ed.buf_mut().move_to_buffer_start();
    start_search(&mut le, true);
    le.read_key(Key::plain('h'));
    assert_eq!(le.ed.search_match(), Some((0, 1)), "match highlighted");
    le.read_key(Key::plain('e'));
    assert_eq!(le.ed.search_match(), Some((0, 2)));
    le.read_key(Key::ctrl('s'));
    assert_eq!(
        le.ed.search_match(),
        Some((12, 14)),
        "next match highlighted"
    );
    // C-g clears the highlight and returns to the start
    le.read_key(Key::ctrl('g'));
    assert_eq!(le.ed.search_match(), None, "highlight cleared on abort");
    assert_eq!(le.point(), 0);
}

#[test]
fn failed_search_clears_highlight() {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert("abc");
    le.ed.buf_mut().move_to_buffer_start();
    start_search(&mut le, true);
    le.read_key(Key::plain('a'));
    assert_eq!(le.ed.search_match(), Some((0, 1)));
    le.read_key(Key::plain('z')); // "az" fails
    assert_eq!(le.ed.search_match(), None, "no match: highlight cleared");
    le.read_key(Key::ctrl('g'));
}

#[test]
fn space_appends_to_query() {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert("hello world");
    le.ed.buf_mut().move_to_buffer_start();
    start_search(&mut le, true);
    for c in "hello".chars() {
        le.read_key(Key::plain(c));
    }
    le.read_key(Key::key(KeyCode::Char(' ')));
    for c in "world".chars() {
        le.read_key(Key::plain(c));
    }
    assert_eq!(le.point(), 0, "matched the full phrase including the space");
    le.read_key(Key::ctrl('g'));
}

#[test]
fn dash_and_angle_bracket_are_literal() {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert("a-b <c>");
    le.ed.buf_mut().move_to_buffer_start();
    start_search(&mut le, true);
    le.read_key(Key::plain('a'));
    assert!(le.read_key(Key::plain('-')), "'-' extends the query");
    le.read_key(Key::plain('b'));
    assert_eq!(le.ed.search_match(), Some((0, 3)), "matched \"a-b\"");
    assert!(le.read_key(Key::plain(' ')), "space extends the query");
    assert!(le.read_key(Key::plain('<')), "'<' extends the query");
    le.read_key(Key::plain('c'));
    assert_eq!(le.ed.search_match(), Some((0, 6)), "matched \"a-b <c\"");
    assert_eq!(le.point(), 0);
    le.read_key(Key::ctrl('g'));
}
