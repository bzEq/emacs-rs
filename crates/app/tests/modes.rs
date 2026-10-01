//! PTY regression tests: Lua-defined commands, major/minor modes, keymaps,
//! and the line-number gutter.

mod common;

use common::{scratch_dir, write_file, write_init, Em};

const INIT: &str = r#"
emacs.define_command("insert-timestamp", function(prefix)
  emacs.insert("T" .. tostring(prefix))
end)
emacs.bind("C-c t", "insert-timestamp")
emacs.define_major_mode("txt-mode", {
  indent = 2,
  keymap = { ["C-c h"] = "insert-timestamp" },
})
emacs.define_minor_mode("my-extra", {
  lighter = "XX",
  keymap = { ["C-c e"] = "insert-timestamp" },
})
emacs.message("lua-config-loaded")
"#;

fn spawn_with_config(init: &str, file_content: &str) -> (Em, std::path::PathBuf) {
    let scratch = scratch_dir();
    write_init(&scratch, init);
    let path = write_file(&scratch, "notes.txt", file_content);
    let path_s = path.to_string_lossy().into_owned();
    let em = Em::spawn_with_scratch(scratch, &[&path_s]);
    (em, path)
}

#[test]
fn lua_global_binding_runs() {
    let (mut em, _) = spawn_with_config(INIT, "plain\n");
    assert!(em.wait_for("lua-config-loaded", 5000), "init.lua ran");
    em.keys(b"\x03t"); // C-c t
    assert!(
        em.wait_for_row(0, "T1plain", 3000),
        "global binding inserts"
    );
    em.quit();
}

#[test]
fn lua_major_mode_local_keymap() {
    let (mut em, _) = spawn_with_config(INIT, "plain\n");
    assert!(em.wait_for("plain", 5000));
    em.m_x("txt-mode");
    assert!(em.wait_for("(txt-mode)", 3000), "Lua major mode active");
    em.keys(b"\x03h"); // C-c h = local binding
    assert!(
        em.wait_for_row(0, "T1plain", 3000),
        "local keymap binding runs"
    );
    em.quit();
}

#[test]
fn lua_minor_mode_keymap_and_lighter() {
    let (mut em, _) = spawn_with_config(INIT, "plain\n");
    assert!(em.wait_for("plain", 5000));
    em.m_x("my-extra-mode");
    assert!(em.wait_for("(fundamental-mode XX)", 3000), "lighter shown");
    em.keys(b"\x03e"); // C-c e = minor mode binding
    assert!(
        em.wait_for_row(0, "T1plain", 3000),
        "minor mode keymap binding runs"
    );
    em.quit();
}

#[test]
fn global_bindings_work_inside_the_minibuffer() {
    let (mut em, _) = spawn_with_config(INIT, "plain\n");
    assert!(em.wait_for("plain", 5000));
    // open find-file; while the minibuffer is active, a global binding
    // (C-c t -> insert-timestamp) runs on the minibuffer input (the
    // current buffer during a read, like Emacs), and the read stays
    em.keys(b"\x18\x06"); // C-x C-f
    assert!(em.wait_for("Find file:", 4000), "minibuffer prompt opened");
    em.keys(b"\x03t"); // C-c t
    assert!(
        em.wait_for_row(23, "T1", 3000),
        "global binding ran on the minibuffer input"
    );
    assert!(
        em.wait_for_row(23, "Find file:", 3000),
        "the minibuffer is still reading"
    );
    em.keys(b"\x07"); // C-g aborts the minibuffer
    assert!(em.wait_for("Quit", 3000));
    em.quit();
}

#[test]
fn minibuffer_follows_global_rebindings() {
    // the user rebinds C-f to forward-word; in the minibuffer the same
    // binding must apply to the minibuffer input (Emacs behavior)
    let init = "emacs.bind(\"C-f\", \"forward-word\")\n";
    let (mut em, _) = spawn_with_config(init, "plain\n");
    assert!(em.wait_for("plain", 5000));
    em.keys(b"\x18\x06"); // C-x C-f
    assert!(em.wait_for("Find file:", 4000), "minibuffer prompt opened");
    em.keys(b"\x01\x0b"); // C-a C-k: clear the pre-filled directory input
    em.type_str("hello world");
    assert!(em.wait_for_row(23, "hello world", 3000), "input typed");
    em.keys(b"\x01"); // C-a -> point at the start of the input
    em.keys(b"\x06"); // C-f -> the user's forward-word (moves past "hello")
    em.keys(b"\x04"); // C-d -> deletes the space after "hello"
    assert!(
        em.wait_for_row(23, "helloworld", 3000),
        "C-f followed the global rebinding (word motion, not char motion)"
    );
    assert!(
        !em.screen.row_text(23).contains("hello world"),
        "the space was deleted: point really moved a whole word"
    );
    // the main buffer is untouched by minibuffer commands
    assert!(em.screen.row_text(0).starts_with("plain"));
    em.keys(b"\x07");
    em.quit();
}

#[test]
fn quit_from_the_minibuffer() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "x\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("x", 5000));
    em.keys(b"\x18\x06"); // C-x C-f
    assert!(em.wait_for("Find file:", 4000), "minibuffer prompt opened");
    em.keys(b"\x18\x03"); // C-x C-c from inside the minibuffer
    assert!(
        em.exit_status().map(|s| s.success()).unwrap_or(false),
        "C-x C-c quit the editor from the minibuffer"
    );
}

#[test]
fn line_numbers_gutter() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "one\ntwo\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("one", 5000));
    em.m_x("line-numbers-mode");
    assert!(em.wait_for("Ln", 3000), "lighter in modeline");
    assert!(em.wait_for_row(0, "   1 one", 3000), "gutter shows line 1");
    assert!(em.wait_for_row(1, "   2 two", 3000), "gutter shows line 2");
    em.quit();
}
