//! PTY regression tests: visual line wrapping (word-wrap).

mod common;

use common::{write_file, Em};

#[test]
fn long_lines_wrap_to_multiple_visual_rows() {
    let em = Em::spawn();
    // 150 chars: wraps into two rows at 80 cols
    let long = "x".repeat(150);
    let path = write_file(&em.scratch, "t.txt", &format!("{long}\nsecond\n"));
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(
        em.wait_for("second", 5000),
        "second line visible below the wrap"
    );
    // the continuation of the long line occupies row 1
    assert!(
        em.screen.row_text(1).starts_with('x'),
        "wrapped continuation rendered on the next visual row"
    );
    em.quit();
}

#[test]
fn isearch_highlights_a_match_on_a_wrapped_row() {
    let em = Em::spawn();
    // the match sits beyond column 80, so it lands on a wrapped row
    let path = write_file(
        &em.scratch,
        "t.txt",
        &format!("{}zzz end\n", "a".repeat(100)),
    );
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("zzz", 5000), "content shown (wrapped)");
    em.keys(b"\x13"); // C-s
    em.type_str("zzz");
    assert!(em.wait_for("I-search: zzz", 3000));
    em.drain();
    assert!(
        em.raw_contains(b"48;5;3m"),
        "match on the wrapped row is highlighted (yellow bg)"
    );
    // the wrapped row contains the match text
    assert!(
        em.screen.text().lines().any(|l| l.contains("zzz")),
        "match visible on a wrapped row"
    );
    em.keys(b"\x07");
    em.quit();
}

#[test]
fn cursor_lands_on_the_wrapped_row() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", &format!("{}tail\n", "x".repeat(100)));
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("tail", 5000));
    em.keys(b"\x05"); // C-e: end of the long line (col 104)
    assert!(
        em.wait_for("C104", 3000),
        "point at the end of the long line"
    );
    // the cursor must be inside the window body (wrapped row 1), not off
    // the modeline
    let (_, y) = em.last_cursor_pos().unwrap_or((0, 0));
    assert!(
        y <= 22,
        "cursor y={y} stays inside the buffer area (on the wrapped row)"
    );
    em.quit();
}
