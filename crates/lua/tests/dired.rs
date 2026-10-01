//! Dired, driven through the real Lua runtime with read-string resumes
//! (ported from the old Rust dired tests).

mod common;

use common::LuaEd;

struct TmpDir(std::path::PathBuf);

impl TmpDir {
    fn new() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("em-lua-dired-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        TmpDir(p)
    }

    fn path(&self) -> String {
        self.0.display().to_string()
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Open dired on `dir` (C-x d semantics): the command first asks for the
/// directory via read-string.
fn open_dired(le: &mut LuaEd, dir: &str) {
    le.command_reading("dired");
    assert!(!le.answer(dir), "dired opens without further prompts");
}

#[test]
fn listing_sorted_dirs_first() {
    let t = TmpDir::new();
    std::fs::write(t.0.join("b.txt"), "x").unwrap();
    std::fs::create_dir(t.0.join("adir")).unwrap();
    std::fs::write(t.0.join("a.txt"), "x").unwrap();
    let mut le = LuaEd::new();
    open_dired(&mut le, &t.path());
    let text = le.text();
    assert!(text.starts_with(&format!("  {}:\n", t.path())));
    assert!(text.contains("adir/"));
    let lines: Vec<&str> = text.lines().collect();
    // header (2 lines) + . .. adir a.txt b.txt
    assert!(lines[2].contains("./"));
    assert!(lines[3].contains("../"));
    assert!(lines[4].contains("adir/"));
    assert!(lines[5].contains("a.txt"));
    assert!(lines[6].contains("b.txt"));
}

#[test]
fn marks_carry_across_refresh() {
    let t = TmpDir::new();
    std::fs::write(t.0.join("f1"), "x").unwrap();
    std::fs::write(t.0.join("f2"), "x").unwrap();
    let mut le = LuaEd::new();
    open_dired(&mut le, &t.path());
    // point to the f1 line (line 4, the first entry after the dot entries)
    le.ed.buf_mut().move_to_line(4);
    le.ed.buf_mut().move_to_line_start();
    le.run("dired-mark");
    assert!(le.text().lines().nth(4).unwrap().starts_with('*'));
    le.run("dired-refresh");
    assert!(
        le.text().lines().nth(4).unwrap().starts_with('*'),
        "marks carry across refresh"
    );
}

#[test]
fn entry_at_point_maps_lines() {
    let t = TmpDir::new();
    std::fs::write(t.0.join("file"), "x").unwrap();
    let mut le = LuaEd::new();
    open_dired(&mut le, &t.path());
    // line 2 = ".", line 3 = "..", line 4 = "file"
    le.ed.buf_mut().move_to_line(4);
    le.ed.buf_mut().move_to_line_start();
    le.run("dired-open");
    // opened the file: buffer name is "file", content "x"
    assert_eq!(le.ed.buf().name(), "file");
    assert_eq!(le.text(), "x");
    assert_eq!(le.ed.buf().mode().name, "fundamental-mode");
}

#[test]
fn open_nested_directory() {
    let t = TmpDir::new();
    std::fs::create_dir(t.0.join("sub")).unwrap();
    let mut le = LuaEd::new();
    open_dired(&mut le, &t.path());
    le.ed.buf_mut().move_to_line(4); // "sub" is the first entry after . and ..
    le.ed.buf_mut().move_to_line_start();
    le.run("dired-open");
    assert!(le.text().starts_with(&format!(
        "  {}:\n",
        t.0.join("sub").canonicalize().unwrap().display()
    )));
}

#[test]
fn rename_entry_via_prompt() {
    let t = TmpDir::new();
    std::fs::write(t.0.join("old.txt"), "x").unwrap();
    let mut le = LuaEd::new();
    open_dired(&mut le, &t.path());
    le.ed.buf_mut().move_to_line(4);
    le.ed.buf_mut().move_to_line_start();
    le.command_reading("dired-rename");
    assert!(!le.answer("new.txt"));
    assert!(std::fs::exists(t.0.join("new.txt")).unwrap());
    assert!(!std::fs::exists(t.0.join("old.txt")).unwrap());
    assert!(le.text().contains("new.txt"), "listing refreshed");
}

#[test]
fn delete_with_confirmation() {
    let t = TmpDir::new();
    std::fs::write(t.0.join("beta.txt"), "bbb").unwrap();
    let mut le = LuaEd::new();
    open_dired(&mut le, &t.path());
    le.ed.buf_mut().move_to_line(4);
    le.ed.buf_mut().move_to_line_start();
    le.command_confirming("dired-delete");
    assert!(!le.answer_yes(true));
    assert!(!std::fs::exists(t.0.join("beta.txt")).unwrap());
    assert!(
        le.text().contains("total 2"),
        "listing refreshed after delete"
    );
}

#[test]
fn delete_declined_keeps_file() {
    let t = TmpDir::new();
    std::fs::write(t.0.join("keep.txt"), "x").unwrap();
    let mut le = LuaEd::new();
    open_dired(&mut le, &t.path());
    le.ed.buf_mut().move_to_line(4);
    le.ed.buf_mut().move_to_line_start();
    le.command_confirming("dired-delete");
    assert!(!le.answer_yes(false));
    assert!(std::fs::exists(t.0.join("keep.txt")).unwrap());
}

#[test]
fn create_directory() {
    let t = TmpDir::new();
    std::fs::create_dir(t.0.join("sub")).unwrap();
    let mut le = LuaEd::new();
    open_dired(&mut le, &t.path());
    le.ed.buf_mut().move_to_line(4);
    le.ed.buf_mut().move_to_line_start();
    le.run("dired-open"); // enter sub
    le.command_reading("dired-create-directory");
    assert!(!le.answer("newdir"));
    assert!(std::fs::exists(t.0.join("sub").join("newdir")).unwrap());
    assert!(le.text().contains("newdir/"));
}

#[test]
fn quit_kills_dired_buffer() {
    let t = TmpDir::new();
    let mut le = LuaEd::new();
    open_dired(&mut le, &t.path());
    let before = le.ed.buffers().len();
    assert!(before >= 2);
    le.run("dired-quit");
    assert_eq!(le.ed.buffers().len(), before - 1, "dired buffer killed");
}

#[test]
fn file_completion_lists_dir_entries() {
    let t = TmpDir::new();
    std::fs::create_dir(t.0.join("sub")).unwrap();
    std::fs::write(t.0.join("alpha.txt"), "a").unwrap();
    std::fs::write(t.0.join("beta.txt"), "b").unwrap();
    std::fs::write(t.0.join(".hidden"), "h").unwrap();
    let mut le = LuaEd::new();
    // ask completion through a find-file prompt
    le.command_reading("find-file");
    let input = format!("{}/a", t.path());
    for c in input.chars() {
        le.ed.minibuffer_insert_char(c);
    }
    let cands = le.ed.update_completion(&input).unwrap();
    assert_eq!(cands, vec![format!("{}/alpha.txt", t.path())]);
    le.abort();
}

#[test]
fn find_file_prompt_offers_relative_completions() {
    let t = TmpDir::new();
    std::fs::write(t.0.join("alpha.txt"), "a").unwrap();
    let mut le = LuaEd::new();
    // the current buffer is a file inside t.0, so relative names complete
    let f = t.0.join("main.txt");
    std::fs::write(&f, "m").unwrap();
    le.startup(Some(&f.display().to_string()));
    le.command_reading("find-file");
    let input = "al";
    for c in input.chars() {
        le.ed.minibuffer_insert_char(c);
    }
    let cands = le.ed.update_completion(input).unwrap();
    assert_eq!(
        cands,
        vec!["alpha.txt"],
        "relative names from the file's dir"
    );
    le.abort();
}

#[test]
fn m_x_completion_prefers_prefix_matches() {
    let mut le = LuaEd::new();
    // completion state is computed by the Lua layer
    le.command_reading("execute-extended-command");
    let input = "des";
    for c in input.chars() {
        le.ed.minibuffer_insert_char(c);
    }
    let cands = le.ed.update_completion(input).unwrap();
    assert_eq!(
        cands,
        vec!["describe-bindings".to_string(), "describe-key".to_string()],
        "prefix matches, sorted"
    );
    le.abort();
}

#[test]
fn m_x_completion_falls_back_to_substring() {
    let mut le = LuaEd::new();
    le.command_reading("execute-extended-command");
    let input = "indow";
    for c in input.chars() {
        le.ed.minibuffer_insert_char(c);
    }
    let cands = le.ed.update_completion(input).unwrap();
    assert!(
        cands.contains(&"delete-window".to_string()),
        "substring fallback finds delete-window"
    );
    le.abort();
}

#[test]
fn yes_no_abort_returns_nil() {
    let t = TmpDir::new();
    std::fs::write(t.0.join("f.txt"), "x").unwrap();
    let mut le = LuaEd::new();
    open_dired(&mut le, &t.path());
    le.ed.buf_mut().move_to_line(4);
    le.ed.buf_mut().move_to_line_start();
    le.command_confirming("dired-delete");
    // C-g: resume with nil
    let outcome = le
        .ed
        .resume_pending(emacs_core::script::ResumeValue::Bool(None))
        .expect("resume");
    assert!(le.ed.finish_command(outcome).is_none());
    assert!(
        std::fs::exists(t.0.join("f.txt")).unwrap(),
        "aborted delete"
    );
    assert!(le.ed.pending().is_none(), "command finished");
}

#[test]
fn read_string_abort_returns_nil() {
    let mut le = LuaEd::new();
    le.command_reading("execute-extended-command");
    let outcome = le
        .ed
        .resume_pending(emacs_core::script::ResumeValue::String(None))
        .expect("resume");
    assert!(le.ed.finish_command(outcome).is_none());
    assert!(le.ed.pending().is_none(), "M-x aborted cleanly");
}
