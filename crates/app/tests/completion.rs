//! PTY regression tests: M-x automatic completion and the --init CLI option.

mod common;

use common::{write_file, Em};

#[test]
fn mx_auto_fills_common_prefix() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "x\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("x", 5000));
    em.keys(b"\x1bx"); // M-x
    em.type_str("des");
    // auto-preview shown: typed "des" + preview "cribe-"
    assert!(em.wait_for_row(23, "M-x des", 3000), "typed input shown");
    em.drain();
    assert!(
        em.screen.row_text(23).contains("cribe-"),
        "LCP preview auto-shown"
    );
    assert!(
        em.wait_for("describe-bindings", 3000),
        "candidates displayed"
    );
    em.quit();
}

#[test]
fn mx_backspace_stays_deleted() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "x\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("x", 5000));
    em.keys(b"\x1bx"); // M-x
    em.type_str("des");
    assert!(em.wait_for_row(23, "M-x des", 3000), "typed input shown");
    em.drain();
    assert!(em.screen.row_text(23).contains("cribe-"), "preview shown");
    em.keys(b"\x7f\x7f"); // two backspaces: "des" -> "d"
    assert!(
        em.wait_for_row(23, "M-x d\u{2588}", 3000),
        "input shrinks; preview does not re-insert"
    );
    em.drain();
    assert!(
        !em.screen.row_text(23).contains("cribe-"),
        "deleted chars stay deleted"
    );
    em.quit();
}

#[test]
fn mx_tab_cycles_and_executes() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "x\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("x", 5000));
    em.keys(b"\x1bx"); // M-x
    em.type_str("des");
    assert!(em.wait_for_row(23, "M-x des", 3000), "typed input shown");
    em.keys(b"\t"); // first TAB accepts the preview ("describe-")
    assert!(
        em.wait_for_row(23, "M-x describe-", 3000),
        "TAB accepts the preview"
    );
    em.keys(b"\t"); // second TAB cycles to the first candidate
    assert!(
        em.wait_for_row(23, "M-x describe-bindings", 3000),
        "second TAB cycles"
    );
    em.keys(b"\r");
    assert!(
        em.wait_for("Global key bindings", 5000),
        "RET runs the cycled command"
    );
    em.quit();
}

#[test]
fn init_option_loads_custom_config() {
    let em = Em::spawn();
    let alt = write_file(
        &em.scratch,
        "alt.lua",
        "emacs.message(\"alt-init-loaded\")\n",
    );
    let alt_s = alt.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&["--init", &alt_s]);
    assert!(em.wait_for("alt-init-loaded", 5000), "--init file ran");
    em.quit();
}

#[test]
fn init_option_missing_file_errors() {
    let em = Em::spawn();
    let missing = em.scratch.join("nope.lua");
    let missing_s = missing.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&["--init", &missing_s]);
    assert!(em.wait_for("cannot open init file", 5000), "error shown");
    em.quit();
}

#[test]
fn unknown_option_exits_2() {
    let mut em = Em::spawn_with_args(&["--bogus"]);
    let status = em.exit_status().unwrap();
    assert_eq!(status.code(), Some(2), "unknown option exits 2");
}

#[test]
fn find_file_completes_names() {
    let em = Em::spawn();
    let d = em.scratch.join("dir");
    std::fs::create_dir(&d).unwrap();
    std::fs::write(d.join("hello.txt"), "hi\n").unwrap();
    std::fs::write(d.join("world.txt"), "wo\n").unwrap();
    let d_s = d.to_string_lossy().into_owned();
    let mut em = Em::spawn();
    assert!(em.wait_for("lines", 5000), "editor ready");
    em.keys(b"\x18\x06"); // C-x C-f
    assert!(em.wait_for("Find file:", 4000), "minibuffer prompt opened");
    em.type_str(&format!("{d_s}/hel"));
    em.expect_row(23, "hel", "typed path shown");
    em.drain();
    assert!(
        em.screen.row_text(23).contains("lo.txt"),
        "completion preview for hello.txt"
    );
    em.keys(b"\r"); // RET accepts the preview
    assert!(em.wait_for("hello.txt", 5000), "file opened");
    assert!(em.wait_for("hi", 3000), "file content shown");
    em.quit();
}

#[test]
fn find_file_tab_cycles_directory_entries() {
    let em = Em::spawn();
    let d = em.scratch.join("dir");
    std::fs::create_dir(&d).unwrap();
    std::fs::write(d.join("alpha.txt"), "a\n").unwrap();
    std::fs::write(d.join("beta.txt"), "b\n").unwrap();
    let d_s = d.to_string_lossy().into_owned();
    let mut em = Em::spawn();
    assert!(em.wait_for("lines", 5000), "editor ready");
    em.keys(b"\x18\x06"); // C-x C-f
    assert!(em.wait_for("Find file:", 4000), "minibuffer prompt opened");
    em.type_str(&format!("{d_s}/"));
    // TAB accepts nothing (LCP empty with both entries), cycles to alpha
    em.keys(b"\t");
    em.expect_row(23, "alpha.txt", "TAB cycles to the first entry");
    em.keys(b"\t");
    em.expect_row(23, "beta.txt", "second TAB cycles to beta");
    em.keys(b"\r");
    assert!(em.wait_for("beta.txt", 5000), "cycled file opened");
    em.quit();
}

#[test]
fn mx_minibuffer_editing_keys() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "x\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("x", 5000));
    em.keys(b"\x1bx"); // M-x
    em.type_str("abc");
    assert!(em.wait_for_row(23, "abc", 3000), "typed input shown");
    // C-b C-b moves the cursor before 'b'; C-d deletes 'b'
    em.keys(b"\x02\x02");
    em.drain();
    em.keys(b"\x04");
    assert!(
        em.wait_for_row(23, "M-x ac", 3000),
        "C-d deleted the char at point"
    );
    assert!(!em.screen.row_text(23).contains("abc"));
    // typing goes in at the cursor
    em.type_str("X");
    assert!(em.wait_for_row(23, "aXc", 3000), "inserted at the cursor");
    // C-f moves right, backspace deletes before the cursor
    em.keys(b"\x06\x7f");
    assert!(
        em.wait_for_row(23, "aX", 3000),
        "C-f then backspace deleted the last char"
    );
    // arrow keys move the cursor
    em.keys(b"\x1b[D");
    em.drain();
    em.type_str("Y");
    assert!(em.wait_for_row(23, "aYX", 3000), "Left arrow + insert");
    em.keys(b"\x07"); // C-g aborts
    assert!(em.wait_for("Quit", 3000));
    em.quit();
}

#[test]
fn mx_minibuffer_history_recall() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "x\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("x", 5000));
    // run M-x forward-char, then M-x again and recall it with C-p
    em.m_x("forward-char");
    assert!(em.wait_for("L1 C1", 3000), "first M-x ran");
    em.keys(b"\x1bx");
    assert!(em.wait_for_row(23, "M-x ", 3000), "second M-x prompt");
    em.keys(b"\x10"); // C-p recalls the history
    assert!(
        em.wait_for_row(23, "M-x forward-char", 3000),
        "C-p recalls the previous input"
    );
    em.keys(b"\r");
    assert!(em.wait_for("L2 C0", 3000), "recalled command ran again");
    // C-n walks forward past the end: input clears
    em.keys(b"\x1bx");
    em.keys(b"\x10\x0e");
    assert!(
        em.wait_for_row(23, "M-x \u{2588}", 3000),
        "C-n back past the end"
    );
    em.keys(b"\x07");
    em.quit();
}

#[test]
fn mx_minibuffer_handles_multibyte_input() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "x\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("x", 5000));
    em.keys(b"\x1bx"); // M-x
    em.type_str("中文");
    assert!(
        em.wait_for("中", 3000) && em.wait_for("文", 3000),
        "multibyte input shown"
    );
    em.keys(b"\x7f"); // backspace removes the last char
    assert!(em.wait_for_row(23, "中", 3000), "first char remains");
    assert!(!em.screen.row_text(23).contains("文"), "last char deleted");
    // the editor is still alive and the minibuffer still works
    em.keys(b"\x07"); // C-g aborts
    assert!(em.wait_for("Quit", 3000));
    em.quit();
}

#[test]
fn find_file_relative_path_shows_absolute_directory() {
    // `em a/b/c.cc` stores a relative path; find-file must still offer
    // the absolute directory (Emacs behavior), so opening a sibling works
    let em = Em::spawn();
    let sub = em.scratch.join("a").join("b");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("c.cc"), "c\n").unwrap();
    std::fs::write(sub.join("d.cc"), "d\n").unwrap();
    let scratch = em.scratch.clone();
    let mut em = Em::spawn_bin_in(
        scratch,
        &common::em_binary(),
        Some(&em.scratch),
        &[],
        &["a/b/c.cc"],
    );
    assert!(em.wait_for("c.cc", 5000), "relative file opened");
    em.keys(b"\x18\x06"); // C-x C-f
    assert!(em.wait_for("Find file:", 4000), "minibuffer prompt opened");
    // the pre-filled directory is absolute
    let abs_dir = format!("{}/a/b/", em.scratch.display());
    assert!(
        em.wait_for_row(23, &abs_dir, 3000),
        "absolute directory pre-filled: {abs_dir}"
    );
    // typing a sibling name resolves against that directory directly
    em.type_str("d.cc");
    em.keys(b"\r");
    assert!(
        em.wait_for("d\n", 5000) || em.wait_for("d.cc", 5000),
        "sibling opened"
    );
    assert!(em.screen.row_text(0).starts_with("d"), "d.cc content shown");
    em.quit();
}

#[test]
fn find_file_defaults_to_buffer_file_directory() {
    let em = Em::spawn();
    let d = em.scratch.join("proj");
    std::fs::create_dir(&d).unwrap();
    std::fs::write(d.join("main.txt"), "main\n").unwrap();
    std::fs::write(d.join("hello.txt"), "hi\n").unwrap();
    let d_s = d.to_string_lossy().into_owned();
    let main_s = format!("{d_s}/main.txt");
    let mut em = Em::spawn_with_args(&[&main_s]);
    assert!(em.wait_for("main", 5000));
    em.keys(b"\x18\x06"); // C-x C-f
    assert!(
        em.wait_for(&format!("Find file: {d_s}/"), 3000),
        "prompt starts in the buffer file's directory"
    );
    em.type_str("hel");
    assert!(em.wait_for_row(23, "hel", 3000), "typed prefix shown");
    em.drain();
    assert!(
        em.screen.row_text(23).contains("lo.txt"),
        "relative completion preview"
    );
    em.keys(b"\r");
    assert!(
        em.wait_for("hi", 5000),
        "hello.txt opened from the file dir"
    );
    em.quit();
}

#[test]
fn find_file_minibuffer_scrolls_long_inputs() {
    let em = Em::spawn();
    let d = em.scratch.join("dir").join("d".repeat(50));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("hello.txt"), "hi\n").unwrap();
    let d_s = d.to_string_lossy().into_owned();
    let mut em = Em::spawn();
    assert!(em.wait_for("lines", 5000), "editor ready");
    em.keys(b"\x18\x06"); // C-x C-f
    assert!(em.wait_for("Find file:", 4000), "minibuffer opened");
    em.type_str(&format!("{d_s}/hel"));
    // the line overflows 80 columns; the minibuffer scrolls so the typed
    // tail, the caret, and the completion preview stay visible
    em.expect_row(23, "hel", "long input tail visible");
    em.drain();
    assert!(
        em.screen.row_text(23).contains("lo.txt"),
        "completion preview stays visible while scrolled"
    );
    em.keys(b"\r");
    assert!(em.wait_for("hi", 5000), "file opened");
    em.quit();
}
