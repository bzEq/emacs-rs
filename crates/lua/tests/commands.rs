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
fn kill_line_with_only_blanks_left_kills_newline() {
    // Emacs: "if no nonblanks there, kill thru newline" — trailing blanks
    // after point do not prevent joining the next line
    let mut le = editor_with("abc   \ndef");
    le.ed.buf_mut().move_to_buffer_start();
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    assert_eq!(le.ed.buf().point(), 3);
    le.run("kill-line");
    assert_eq!(le.text(), "abcdef", "killed the blanks and the newline");
}

#[test]
fn kill_line_stops_before_nonblank_rest() {
    let mut le = editor_with("foo bar baz\nnext");
    le.ed.buf_mut().move_to_buffer_start();
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    assert_eq!(le.ed.buf().point(), 4);
    le.run("kill-line");
    assert_eq!(le.text(), "foo \nnext", "nonblank rest: killed to eol only");
}

#[test]
fn kill_line_prefix_kills_several_lines() {
    let mut le = editor_with("one\ntwo\nthree\n");
    le.ed.buf_mut().move_to_buffer_start();
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    le.run("digit-argument-2");
    le.run("kill-line");
    assert_eq!(le.text(), "othree\n", "killed through two line endings");
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
fn mode_switch_installs_keymap() {
    let mut le = editor_with("fn main() {}");
    le.run("rust-mode");
    assert_eq!(le.ed.buf().mode().name, "rust-mode");
    le.run("fundamental-mode");
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
fn keyboard_quit_deactivates_but_keeps_the_mark() {
    let mut le = editor_with("hello world");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("set-mark-command");
    le.ed
        .buf_mut()
        .move_char(emacs_core::buffer::Direction::Forward);
    assert_eq!(le.ed.buf().region(), Some((0, 1)), "region active");
    le.run("keyboard-quit");
    assert_eq!(le.ed.buf().region(), None, "C-g deactivates the region");
    assert_eq!(
        le.ed.buf().mark(),
        Some(0),
        "the mark position survives C-g"
    );
    // C-x C-x reactivates it
    le.run("exchange-point-and-mark");
    assert_eq!(le.ed.buf().region(), Some((0, 1)), "C-x C-x reactivates");
}

#[test]
fn set_mark_twice_at_point_deactivates() {
    let mut le = editor_with("hello");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("set-mark-command");
    assert!(le.ed.buf().mark_active());
    le.run("set-mark-command");
    assert!(
        !le.ed.buf().mark_active(),
        "second C-SPC at point deactivates"
    );
    assert_eq!(le.ed.buf().region(), None);
}

#[test]
fn kill_region_deactivates_the_mark() {
    let mut le = editor_with("hello");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("set-mark-command");
    le.ed.buf_mut().move_to_buffer_end();
    assert_eq!(le.ed.buf().region(), Some((0, 5)));
    le.run("kill-region");
    assert_eq!(le.ed.buf().region(), None, "kill-region deactivates");
    assert_eq!(le.text(), "");
}

#[test]
fn kill_ring_save_copies_without_deleting() {
    let mut le = editor_with("hello world");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("set-mark-command");
    le.ed
        .buf_mut()
        .move_word(emacs_core::buffer::Direction::Forward);
    le.run("kill-ring-save");
    assert_eq!(le.text(), "hello world", "the region is not deleted");
    assert_eq!(le.point(), 5, "point stays at the end of the region");
    assert!(!le.ed.buf().mark_active(), "the mark is deactivated");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("yank");
    assert_eq!(
        le.text(),
        "hello worldhello",
        "the copy is in the kill ring"
    );
}

#[test]
fn kill_ring_save_reports_what_was_copied() {
    let mut le = editor_with("hello\nworld");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("set-mark-command");
    le.ed.buf_mut().set_point(6); // include the newline in the region
    le.run("kill-ring-save");
    assert_eq!(
        le.ed.echo(),
        Some("Copied text from \"hello^J\""),
        "control characters are made visible"
    );
}

#[test]
fn kill_ring_save_appends_after_a_kill_command() {
    let mut le = editor_with("one two three");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("set-mark-command");
    le.ed.buf_mut().set_point(8);
    le.run("kill-line"); // kill ring: "three"; the mark stays at 0
    assert_eq!(le.text(), "one two ");
    le.run("kill-ring-save"); // copies "one two " and appends to "three"
    le.ed.buf_mut().move_to_buffer_end();
    le.run("yank");
    assert_eq!(le.text(), "one two threeone two ");
}

#[test]
fn kill_ring_save_prepends_for_a_backward_region() {
    let mut le = editor_with("one two three");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("set-mark-command");
    le.ed.buf_mut().set_point(8);
    le.run("backward-kill-word"); // kills "two "; point now before mark
    assert_eq!(le.text(), "one three");
    assert!(le.point() < le.ed.buf().mark().unwrap());
    le.run("kill-ring-save"); // copies "three" before the last kill
    assert_eq!(le.ed.echo(), Some("Copied text until \"three\""));
    le.ed.buf_mut().move_to_buffer_end();
    le.run("yank");
    assert_eq!(le.text(), "one threethreetwo ");
}

#[test]
fn kill_region_prepends_for_a_backward_region() {
    let mut le = editor_with("one two three");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("set-mark-command");
    le.ed.buf_mut().set_point(0);
    le.run("kill-word"); // kills "one"; point now before mark
    assert_eq!(le.text(), " two three");
    assert!(le.point() < le.ed.buf().mark().unwrap());
    le.run("kill-region"); // kills " two three", prepending to "one"
    assert_eq!(le.text(), "");
    le.run("yank");
    assert_eq!(le.text(), " two threeone");
}
