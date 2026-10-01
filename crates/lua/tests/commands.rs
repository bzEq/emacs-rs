//! Editing commands, undo, the kill ring, and prefix arguments, driven
//! through the real Lua runtime (ported from the old Rust command tests).

mod common;

use common::LuaEd;

fn editor_with(text: &str) -> LuaEd {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert(text);
    le
}

#[test]
fn kill_line_simple() {
    let mut le = editor_with("hello\nworld");
    le.ed.buf_mut().move_to_buffer_start();
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    le.run("kill-line");
    assert_eq!(le.text(), "h\nworld");
}

#[test]
fn kill_line_at_eol_kills_newline() {
    let mut le = editor_with("hello\nworld");
    le.ed.buf_mut().move_to_buffer_start();
    le.ed.buf_mut().move_to_line_end();
    le.run("kill-line");
    assert_eq!(le.text(), "helloworld");
}

#[test]
fn consecutive_kills_accumulate() {
    let mut le = editor_with("abc def");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("kill-word");
    le.run("kill-word");
    // the kill ring entry is Lua state; check it through yank
    le.run("yank");
    assert_eq!(le.text(), "abc def", "accumulated kill yanks back as one");
}

#[test]
fn yank_and_pop_single_entry_errors() {
    let mut le = editor_with("hello");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("set-mark-command");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("kill-region");
    le.run("yank");
    assert_eq!(le.text(), "hello");
    // a single kill-ring entry: yank-pop shows an error
    le.run("yank-pop");
    assert_eq!(
        le.ed.echo(),
        Some("Only one element in the kill ring"),
        "yank-pop errors on a single entry"
    );
    assert_eq!(le.text(), "hello");
}

#[test]
fn yank_pop_rotates() {
    let mut le = editor_with("one two");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("kill-word"); // "one"
    le.run("forward-char"); // breaks the kill sequence
    le.run("kill-word"); // "two"
    assert_eq!(le.text(), " ");
    le.run("yank");
    assert_eq!(le.text(), " two", "most recent kill first");
    le.run("yank-pop");
    assert_eq!(le.text(), " one", "yank-pop rotates to the previous kill");
}

#[test]
fn undo_insert_group() {
    let mut le = editor_with("ab");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("newline");
    le.run("newline");
    assert_eq!(le.text(), "ab\n\n");
    le.run("undo");
    assert_eq!(le.text(), "ab", "one undo removes the whole newline group");
}

#[test]
fn undo_group_and_delete() {
    let mut le = editor_with("abcd");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("delete-char");
    assert_eq!(le.text(), "bcd");
    le.run("undo");
    assert_eq!(le.text(), "abcd", "undo restores the deleted char");
    assert_eq!(le.point(), 1);
}

#[test]
fn region_kill() {
    let mut le = editor_with("hello world");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("set-mark-command");
    le.ed
        .buf_mut()
        .move_word(emacs_core::buffer::Direction::Forward);
    le.run("kill-region");
    assert_eq!(le.text(), " world");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("yank");
    assert_eq!(le.text(), "hello world");
}

#[test]
fn prefix_arg_motion() {
    let mut le = editor_with("abcdef");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("digit-argument-3");
    le.run("forward-char");
    assert_eq!(le.point(), 3);
    // prefix consumed by the motion command
    le.run("forward-char");
    assert_eq!(le.point(), 4);
}

#[test]
fn digits_beat_universal() {
    let content: String = (1..=10).map(|i| format!("line {i}\n")).collect();
    let mut le = editor_with(&content);
    le.ed.buf_mut().move_to_buffer_start();
    le.run("universal-argument");
    le.run("digit-argument-5");
    le.run("goto-line");
    assert_eq!(le.ed.buf().line_of_point(), 4, "digits win over C-u");
}

#[test]
fn universal_argument_quadruples() {
    let mut le = editor_with("abcd");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("universal-argument");
    le.run("delete-char");
    assert_eq!(le.text(), "", "C-u deletes 4 chars");
}

#[test]
fn self_insert_repeats_with_prefix() {
    let mut le = editor_with("");
    le.run("digit-argument-3");
    le.command_extra("self-insert-command", Some('Z'));
    assert_eq!(le.text(), "ZZZ");
}

#[test]
fn goto_line_via_prefix_arg() {
    let content: String = (1..=10).map(|i| format!("line {i}\n")).collect();
    let mut le = editor_with(&content);
    le.ed.buf_mut().move_to_buffer_start();
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    le.run("digit-argument-5");
    le.run("goto-line");
    assert_eq!(le.ed.buf().line_of_point(), 4, "line 5 (0-based 4)");
    assert_eq!(le.ed.buf().column(), 0);
    assert_eq!(le.ed.buf().mark(), Some(1), "mark at the previous position");
}

#[test]
fn goto_line_clamps_to_last_line() {
    let mut le = editor_with("a\nb\n");
    le.run("digit-argument-9");
    le.run("digit-argument-9"); // 99
    le.run("goto-line");
    assert_eq!(le.ed.buf().line_of_point(), le.ed.buf().len_lines() - 1);
}

#[test]
fn exchange_point_and_mark() {
    let mut le = editor_with("hello world");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("set-mark-command");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("exchange-point-and-mark");
    assert_eq!(le.point(), 0);
    assert_eq!(le.ed.buf().mark(), Some(11));
}

#[test]
fn mode_switch_installs_keymap_and_reparses() {
    let mut le = editor_with("fn main() {}");
    le.run("rust-mode");
    assert_eq!(le.ed.buf().mode().name, "rust-mode");
    assert!(le.ed.buf().syntax_dirty());
    le.ed.refresh_syntax_current();
    assert!(le.ed.buf().syntax().is_some());
    le.run("fundamental-mode");
    assert!(le.ed.buf().syntax().is_none());
    assert_eq!(le.ed.buf().mode().name, "fundamental-mode");
}

#[test]
fn line_numbers_minor_mode() {
    let mut le = editor_with("x");
    le.run("line-numbers-mode");
    assert!(le
        .ed
        .minor_mode_enabled(le.ed.selected_buffer_index(), "line-numbers"));
    le.run("line-numbers-mode");
    assert!(!le
        .ed
        .minor_mode_enabled(le.ed.selected_buffer_index(), "line-numbers"));
}

#[test]
fn self_insert_rejects_read_only_buffer() {
    let mut le = editor_with("x");
    le.ed.buf_mut().set_read_only(true);
    le.command_extra("self-insert-command", Some('Z'));
    assert_eq!(le.text(), "x", "nothing inserted");
    assert_eq!(le.ed.echo(), Some("Buffer is read-only"));
}

#[test]
fn negative_prefix_inverts_motion() {
    let mut le = editor_with("abcdef");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("negative-argument");
    le.run("forward-char");
    assert_eq!(le.point(), 5, "M-- C-f moves backward");
    le.run("negative-argument");
    le.run("backward-char");
    assert_eq!(le.point(), 6, "M-- C-b moves forward");
}

#[test]
fn negative_prefix_inverts_delete() {
    let mut le = editor_with("abcdef");
    le.ed.buf_mut().move_to_buffer_start();
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    le.run("negative-argument");
    le.run("delete-char");
    assert_eq!(le.text(), "bcdef", "M-- C-d deletes backward");
}

#[test]
fn kill_after_yank_pop_starts_new_entry() {
    let mut le = editor_with("aaa");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("kill-word"); // "aaa" -> ring entry 1
    le.ed.buf_mut().insert("bbb");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("set-mark-command"); // breaks the kill sequence (point stays)
    le.run("kill-word"); // "bbb" -> ring entry 2
    le.run("yank");
    le.run("yank-pop"); // ring rotates back to entry 1 ("aaa")
    assert_eq!(le.text(), "aaa");
    le.ed.buf_mut().move_to_buffer_end();
    le.ed.buf_mut().insert(" ccc");
    le.run("backward-kill-word"); // kills "ccc": must start a NEW entry
    assert_eq!(le.text(), "aaa ", "only the last word is killed");
    le.run("yank");
    assert_eq!(le.text(), "aaa ccc", "yanks the fresh kill");
    le.run("yank-pop"); // -> "bbb"
    assert_eq!(le.text(), "aaa bbb");
    le.run("yank-pop"); // -> "aaa"
    assert_eq!(
        le.text(),
        "aaa aaa",
        "the ring is aaa/bbb/ccc; appending to an older entry would corrupt it"
    );
}

#[test]
fn edit_commands_reject_read_only_buffer() {
    let mut le = editor_with("hello world");
    le.ed.buf_mut().move_to_buffer_start();
    le.ed.buf_mut().set_read_only(true);
    let before = le.text();
    le.run("kill-word");
    assert_eq!(le.text(), before, "kill-word is blocked");
    assert!(
        le.ed.echo().is_some_and(|e| e.contains("read-only")),
        "an error is reported: {:?}",
        le.ed.echo()
    );
    le.run("delete-char");
    assert_eq!(le.text(), before, "delete-char is blocked");
    le.run("yank");
    assert_eq!(le.text(), before, "yank is blocked");
}

#[test]
fn goto_line_whitespace_input_aborts() {
    let mut le = editor_with("a\nb\nc\n");
    le.ed.buf_mut().move_to_buffer_start();
    le.command_reading("goto-line");
    le.resume(emacs_core::script::ResumeValue::String(Some(
        "   ".to_string(),
    )));
    assert_eq!(
        le.ed.buf().line_of_point(),
        0,
        "whitespace-only input aborts"
    );
    assert!(le.ed.pending().is_none(), "command finished");
}
