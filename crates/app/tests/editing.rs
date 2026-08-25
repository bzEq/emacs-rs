//! PTY regression tests: basic editing, motion, undo, save, quit.

mod common;

use common::{write_file, Em};

#[test]
fn insert_undo_via_cxu() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "hello world\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("hello world", 5000), "file content visible");
    em.keys(b"XY");
    assert!(
        em.wait_for_row(0, "XYhello world", 3000),
        "inserted at point"
    );
    em.keys(b"\x18u"); // C-x u = undo
    assert!(
        em.wait_for_row(0, "hello world", 3000),
        "undo restores line"
    );
    assert!(!em.screen.row_text(0).contains("XYhello"));
    em.quit();
}

#[test]
fn motion_updates_modeline() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "one\ntwo\nthree\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("one", 5000));
    em.keys(b"\x0e\x0e"); // C-n C-n
    assert!(em.wait_for("L3 C0", 3000), "point on line 3");
    em.keys(b"\x05"); // C-e
    assert!(em.wait_for("L3 C5", 3000), "end of 'three'");
    em.quit();
}

#[test]
fn save_writes_to_disk() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "abc\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("abc", 5000));
    em.type_str("ZZ");
    em.keys(b"\x18\x13"); // C-x C-s
    assert!(em.wait_for("Wrote", 5000), "save message");
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(saved, "ZZabc\n");
    em.quit();
}

#[test]
fn modified_quit_prompts() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "abc\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("abc", 5000));
    em.type_str("X");
    em.keys(b"\x18\x03"); // C-x C-c
    assert!(
        em.wait_for("save it? (y/n)", 5000),
        "modified buffer prompts on quit"
    );
    em.keys(b"n");
    assert!(em.exit_status().map(|s| s.success()).unwrap_or(false));
    // not saved
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "abc\n");
}

#[test]
fn kill_line_and_yank() {
    let mut em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "hello world\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("hello world", 5000));
    // point is at 0; kill to end of line, then yank
    em.keys(b"\x0b"); // C-k
    assert!(em.wait_for_row(0, "", 3000), "line killed");
    em.keys(b"\x19"); // C-y
    assert!(em.wait_for_row(0, "hello world", 3000), "yanked back");
    em.quit();
}

#[test]
fn cursor_stays_out_of_the_modeline_at_buffer_end() {
    let em = Em::spawn();
    let content: String = (0..40).map(|i| format!("line {i}\n")).collect();
    let path = write_file(&em.scratch, "t.txt", &content);
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("line 0", 5000));
    em.keys(b"\x1b>"); // M-> to the last line
    assert!(em.wait_for("line 39", 3000), "last line visible");
    std::thread::sleep(std::time::Duration::from_millis(200));
    let (_, y) = em.last_cursor_pos().unwrap_or((0, 0));
    // 24-row terminal: buffer rows 1..=22, modeline 23, echo 24
    assert!(
        y <= 22,
        "cursor y={y} must stay inside the buffer area, off the modeline"
    );
    em.quit();
}
