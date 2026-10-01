//! PTY regression tests for signal handling: SIGTERM/SIGHUP quit cleanly
//! (restoring the terminal), and SIGINT acts as C-g.

mod common;

use common::{scratch_dir, write_file, Em};

#[test]
fn sigterm_quits_cleanly() {
    let scratch = scratch_dir();
    let path = write_file(&scratch, "t.txt", "hello\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_scratch(scratch, &[&path_s]);
    assert!(em.wait_for("hello", 5000), "editor ready");
    em.signal(libc::SIGTERM);
    let status = em.exit_status().expect("em exits on SIGTERM");
    assert_eq!(status.code(), Some(0), "clean exit after SIGTERM");
    // the terminal is restored: after the child ends the master sees the
    // alternate-screen teardown
    assert!(
        em.raw_contains(b"\x1b[?1049l"),
        "leave-alternate-screen is emitted on SIGTERM"
    );
}

#[test]
fn sigint_aborts_pending_read() {
    let mut em = Em::spawn();
    assert!(em.wait_for("lines", 5000), "editor ready");
    em.keys(b"\x1bx"); // open the M-x read-string prompt
    assert!(em.wait_for("M-x", 5000), "minibuffer prompt shown");
    em.type_str("abc");
    assert!(em.wait_for("abc", 2000), "input shown");
    em.signal(libc::SIGINT);
    assert!(em.wait_for("Quit", 2000), "SIGINT acts as C-g");
    // the editor is still alive and usable
    em.type_str("x");
    assert!(em.wait_for("x", 2000), "editor still responsive");
    em.signal(libc::SIGTERM);
    let _ = em.exit_status();
}
