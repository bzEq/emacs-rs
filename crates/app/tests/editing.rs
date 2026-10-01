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
fn new_file_opens_buffer_and_save_creates_it() {
    // `em newfile.txt` with a missing file: the buffer opens anyway, and
    // C-x C-s creates the file on disk.
    let em = Em::spawn();
    let target = em.scratch.join("newfile.txt");
    let target_s = target.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&target_s]);
    assert!(
        em.wait_for("newfile.txt", 5000),
        "buffer for the missing file opens"
    );
    assert!(!target.exists(), "file not created yet");
    em.type_str("hello new file");
    em.keys(b"\x18\x13"); // C-x C-s
    assert!(em.wait_for("Wrote", 3000), "save message");
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "hello new file",
        "the file now exists with the buffer content"
    );
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
    let em = Em::spawn();
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
fn region_is_highlighted_after_set_mark() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "hello world\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("hello world", 5000));
    em.keys(b"\x00"); // C-SPC: set the mark
    assert!(em.wait_for("Mark set", 3000));
    em.keys(b"\x06\x06\x06"); // C-f C-f C-f: point moves 3 chars
    em.drain();
    // the region between point and mark gets the light-blue background
    assert!(
        em.raw_contains(b"48;5;12m"),
        "region highlight escape present (48;5;12 = LightBlue bg)"
    );
    // C-g quits: the region highlight is cancelled
    let marker = em.raw.len();
    em.keys(b"\x07"); // C-g
    assert!(em.wait_for("Quit", 3000));
    em.drain();
    assert!(
        !em.raw[marker..].windows(7).any(|w| w == b"48;5;12m"),
        "C-g cancelled the region highlight"
    );
    em.quit();
}

#[test]
fn set_mark_twice_deactivates() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "hello world\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("hello world", 5000));
    em.keys(b"\x00\x06\x06"); // C-SPC C-f C-f: mark at 0, point at 2, active
    em.drain();
    assert!(em.raw_contains(b"48;5;12m"), "region highlighted");
    em.keys(b"\x02\x02"); // point back to 0 (== mark)
    let marker = em.raw.len();
    em.keys(b"\x00"); // C-SPC with an active mark at point -> deactivates
    assert!(em.wait_for("Mark deactivated", 3000));
    em.drain();
    assert!(
        !em.raw[marker..].windows(7).any(|w| w == b"48;5;12m"),
        "second C-SPC at point deactivated the mark"
    );
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

#[test]
fn goto_line_via_mgmg_prompt() {
    let em = Em::spawn();
    let content: String = (1..=10).map(|i| format!("line {i}\n")).collect();
    let path = write_file(&em.scratch, "t.txt", &content);
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("line 1", 5000));
    em.keys(b"\x1bg\x1bg"); // M-g M-g
    assert!(em.wait_for("Goto line", 3000), "prompt shown");
    em.type_str("5");
    em.keys(b"\r");
    assert!(em.wait_for("L5 C0", 3000), "point on line 5, column 0");
    em.quit();
}

#[test]
fn goto_line_via_prefix_arg() {
    let em = Em::spawn();
    let content: String = (1..=10).map(|i| format!("line {i}\n")).collect();
    let path = write_file(&em.scratch, "t.txt", &content);
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("line 1", 5000));
    em.keys(b"\x15\x1b5\x1bg\x1bg"); // C-u M-5 M-g M-g
    assert!(
        em.wait_for("L5 C0", 3000),
        "prefix arg 5 jumps without prompting"
    );
    em.quit();
}
