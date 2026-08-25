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
