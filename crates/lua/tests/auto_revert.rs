//! Auto-revert policy, driven through the real Lua runtime: Rust reports
//! changed buffer ids via inotify; Lua reverts unmodified buffers and
//! warns about modified ones.

mod common;

use common::LuaEd;

struct TmpFile(std::path::PathBuf);

impl TmpFile {
    fn new(content: &str) -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("em-auto-revert-{}-{n}", std::process::id()));
        std::fs::write(&p, content).unwrap();
        TmpFile(p)
    }

    fn write(&self, content: &str) {
        // ensure the mtime (millisecond resolution) moves forward
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&self.0, content).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    fn path(&self) -> String {
        self.0.display().to_string()
    }
}

impl Drop for TmpFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn file_changed_reverts_unmodified_buffer() {
    let f = TmpFile::new("one\n");
    let mut le = LuaEd::new();
    le.startup(Some(&f.path()));
    assert_eq!(le.text(), "one\n");
    f.write("two\n");
    let id = le.ed.selected_buffer_id();
    le.ed.notify_file_changes(&[id]).unwrap();
    assert_eq!(le.text(), "two\n", "reverted to the on-disk content");
    assert!(!le.ed.buf().modified());
    // a second notification with no further change is a no-op
    le.ed.notify_file_changes(&[id]).unwrap();
    assert_eq!(le.text(), "two\n");
}

#[test]
fn file_changed_warns_for_modified_buffer() {
    let f = TmpFile::new("one\n");
    let mut le = LuaEd::new();
    le.startup(Some(&f.path()));
    le.ed.buf_mut().move_to_buffer_start();
    le.ed.buf_mut().insert("X"); // modified: "Xone\n"
    f.write("two\n");
    let id = le.ed.selected_buffer_id();
    le.ed.notify_file_changes(&[id]).unwrap();
    assert_eq!(le.text(), "Xone\n", "a modified buffer is not overwritten");
    assert!(
        le.ed
            .echo()
            .unwrap_or_default()
            .ends_with("changed on disk"),
        "warning shown: {:?}",
        le.ed.echo()
    );
}

#[test]
fn auto_revert_can_be_disabled() {
    let f = TmpFile::new("one\n");
    let mut le = LuaEd::new();
    le.startup(Some(&f.path()));
    le.run("global-auto-revert-mode"); // toggle off
    f.write("two\n");
    let id = le.ed.selected_buffer_id();
    le.ed.notify_file_changes(&[id]).unwrap();
    assert_eq!(le.text(), "one\n", "disabled: no revert");
}

#[test]
fn revert_buffer_command_reloads() {
    let f = TmpFile::new("one\n");
    let mut le = LuaEd::new();
    le.startup(Some(&f.path()));
    le.ed.buf_mut().move_to_buffer_start();
    le.ed.buf_mut().insert("X"); // modified
    f.write("two\n");
    // modified buffer: the command asks for confirmation
    le.command_confirming("revert-buffer");
    assert!(!le.answer_yes(true));
    assert_eq!(le.text(), "two\n", "confirmed revert reloaded the file");
    assert!(!le.ed.buf().modified());
}

#[test]
fn save_updates_the_recorded_stat() {
    let f = TmpFile::new("one\n");
    let mut le = LuaEd::new();
    le.startup(Some(&f.path()));
    le.ed.buf_mut().move_to_buffer_end();
    le.ed.buf_mut().insert("edited\n");
    le.run("save-buffer");
    // our own save fires no revert: the recorded stat was refreshed
    let id = le.ed.selected_buffer_id();
    le.ed.notify_file_changes(&[id]).unwrap();
    assert_eq!(le.text(), "one\nedited\n", "own save is not reverted");
    assert!(
        !le.ed.echo().unwrap_or_default().starts_with("Reverted"),
        "no revert message"
    );
}
