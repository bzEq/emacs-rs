//! PTY regression tests: global-auto-revert-mode — an external change to a
//! buffer's file is detected via inotify and the buffer is reverted (or a
//! warning is shown when the buffer is modified).

mod common;

use common::{write_file, Em};

#[test]
fn external_change_reverts_the_buffer() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "one\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("one", 5000));
    // another editor writes the file
    std::fs::write(&path, "two\n").unwrap();
    assert!(
        em.wait_for("two", 5000),
        "buffer reverted to the new content"
    );
    assert!(em.wait_for("Reverted", 3000), "revert message shown");
    em.quit();
}

#[test]
fn modified_buffer_warns_instead_of_reverting() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "one\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("one", 5000));
    em.type_str("X"); // buffer modified: "Xone\n"
    assert!(em.wait_for("Xone", 3000));
    std::fs::write(&path, "two\n").unwrap();
    assert!(
        em.wait_for("changed on disk", 5000),
        "warning shown for a modified buffer"
    );
    assert!(
        em.screen.row_text(0).starts_with("Xone"),
        "modified buffer content was NOT overwritten"
    );
    em.quit();
}

#[test]
fn revert_buffer_command() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "one\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("one", 5000));
    // disable auto-revert so the manual command is what does the work
    em.m_x("global-auto-revert-mode");
    assert!(em.wait_for("Auto-revert disabled", 3000));
    std::fs::write(&path, "two\n").unwrap();
    // Wait for a full event-loop interval; nothing should change.
    assert!(em.wait_until(1200, |em| {
        em.drain();
        em.screen.row_text(0).starts_with("one")
    }));
    assert!(
        em.screen.row_text(0).starts_with("one"),
        "auto-revert disabled: buffer unchanged"
    );
    em.m_x("revert-buffer");
    assert!(em.wait_for("two", 3000), "manual revert-buffer reloaded");
    em.quit();
}
