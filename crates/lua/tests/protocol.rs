//! The Lua-facing protocol: user commands, bindings, hooks, nested
//! execute, and coroutine-based synchronous reads.

mod common;

use common::LuaEd;

const USER_INIT: &str = r#"
emacs.define_command("say-hello", function(prefix)
  emacs.insert("hi" .. tostring(prefix))
end)
emacs.bind("C-c h", "say-hello")

emacs.define_command("ask-and-insert", function()
  local s = emacs.read_string("What? ", nil)
  if s then
    emacs.insert("[" .. s .. "]")
  end
end)

emacs.define_command("confirm-delete", function()
  local yes = emacs.read_yes_no("Really? (y/n)")
  if yes == true then
    emacs.insert("gone")
  elseif yes == false then
    emacs.insert("kept")
  end
end)

emacs.define_command("two-reads", function()
  local a = emacs.read_string("First: ", nil)
  local b = emacs.read_string("Second: ", nil)
  emacs.insert((a or "nil") .. "," .. (b or "nil"))
end)

emacs.define_command("nested-execute", function()
  emacs.execute("say-hello")
  emacs.insert("!")
end)

local saved = 0
emacs.add_hook("before_save", function()
  saved = saved + 1
end)
emacs.define_command("hook-count", function()
  emacs.insert(tostring(saved))
end)

emacs.define_major_mode("lispy-mode", {
  indent = 2,
  keymap = { ["C-c z"] = "say-hello" },
})
emacs.define_minor_mode("fancy", {
  lighter = "FY",
  keymap = { ["C-c y"] = "say-hello" },
})
"#;

fn with_init() -> LuaEd {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let mut le = LuaEd::new();
    let dir = std::env::temp_dir().join(format!("em-lua-init-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("init.lua");
    std::fs::write(&path, USER_INIT).unwrap();
    le.load(&path);
    le
}

#[test]
#[allow(clippy::unnecessary_to_owned)]
fn user_command_and_global_binding() {
    let mut le = with_init();
    // keymap lookup resolves C-c h to the user command
    le.ed.push_key(emacs_core::key::Key::ctrl('c'));
    assert_eq!(
        le.ed.lookup_key(&le.ed.pending_keys().to_vec()),
        emacs_core::keymap::Lookup::Prefix
    );
    le.ed.push_key(emacs_core::key::Key::plain('h'));
    assert_eq!(
        le.ed.lookup_key(&le.ed.pending_keys().to_vec()),
        emacs_core::keymap::Lookup::Command("say-hello".into())
    );
    le.ed.clear_pending_keys();
    le.run("say-hello");
    assert_eq!(le.text(), "hi1", "user command receives prefix 1");
}

#[test]
fn read_string_protocol() {
    let mut le = with_init();
    le.command_reading("ask-and-insert");
    assert!(!le.answer("xyz"));
    assert_eq!(le.text(), "[xyz]");
}

#[test]
fn minibuffer_is_a_registered_buffer_with_its_mode() {
    let mut le = LuaEd::new();
    le.command_reading("find-file");
    let id = le.ed.current_buffer_id();
    assert_ne!(id, le.ed.selected_buffer_id(), "the minibuffer is current");
    let buf = le
        .ed
        .buffers()
        .iter()
        .find(|b| b.id == id)
        .expect("the minibuffer is a real buffer");
    assert_eq!(buf.name(), " *Minibuf*");
    assert_eq!(buf.mode().name, "minibuffer-mode");
    assert!(
        buf.local_keymap().is_some(),
        "minibuffer-mode installs a buffer-local keymap"
    );
    assert!(
        !buf.modified(),
        "the pre-filled input is not a modification"
    );

    // its local keymap is consulted by the normal lookup
    le.ed.push_key(emacs_core::key::Key::ctrl('p'));
    let seq = le.ed.pending_keys().to_vec();
    assert_eq!(
        le.ed.lookup_key(&seq),
        emacs_core::keymap::Lookup::Command("minibuf-previous-history".into())
    );
    le.ed.clear_pending_keys();

    le.abort();
    assert!(
        !le.ed.buffers().iter().any(|b| b.id == id),
        "finishing the read kills the minibuffer buffer"
    );
}

#[test]
fn read_string_abort_inserts_nothing() {
    let mut le = with_init();
    le.command_reading("ask-and-insert");
    let outcome = le
        .ed
        .resume_pending(emacs_core::script::ResumeValue::String(None))
        .expect("resume");
    assert!(le.ed.finish_command(outcome).is_none());
    assert_eq!(le.text(), "", "nil answer: nothing inserted");
}

#[test]
fn read_yes_no_protocol() {
    let mut le = with_init();
    le.command_confirming("confirm-delete");
    assert!(!le.answer_yes(true));
    assert_eq!(le.text(), "gone");

    let mut le = with_init();
    le.command_confirming("confirm-delete");
    assert!(!le.answer_yes(false));
    assert_eq!(le.text(), "kept");
}

#[test]
fn consecutive_reads_suspend_between() {
    let mut le = with_init();
    le.command_reading("two-reads");
    assert!(le.answer("a"), "a second read-string is pending");
    assert!(!le.answer("b"));
    assert_eq!(le.text(), "a,b");
}

#[test]
fn nested_execute_runs_inline() {
    let mut le = with_init();
    le.run("nested-execute");
    assert_eq!(le.text(), "hi1!");
}

#[test]
fn hooks_run_on_save() {
    let t = std::env::temp_dir().join(format!("em-lua-hook-{}", std::process::id()));
    std::fs::create_dir_all(&t).unwrap();
    let f = t.join("f.txt");
    std::fs::write(&f, "x").unwrap();
    let mut le = with_init();
    le.startup(Some(&f.display().to_string()));
    le.run("save-buffer");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("hook-count");
    assert!(le.text().ends_with('1'), "before_save hook ran once");
    le.run("save-buffer");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("hook-count");
    assert!(le.text().ends_with("x12") || le.text().ends_with("12"));
    let _ = std::fs::remove_dir_all(&t);
}

#[test]
#[allow(clippy::unnecessary_to_owned)]
fn lua_major_mode_keymap_and_indent() {
    let mut le = with_init();
    le.run("lispy-mode");
    assert_eq!(le.ed.buf().mode().name, "lispy-mode");
    // local keymap: C-c z resolves to say-hello
    le.ed.push_key(emacs_core::key::Key::ctrl('c'));
    le.ed.push_key(emacs_core::key::Key::plain('z'));
    assert_eq!(
        le.ed.lookup_key(&le.ed.pending_keys().to_vec()),
        emacs_core::keymap::Lookup::Command("say-hello".into())
    );
    le.ed.clear_pending_keys();
    // indent unit from the Lua mode opts
    le.ed.buf_mut().insert("{");
    le.run("newline-and-indent");
    assert_eq!(le.text(), "{\n  ", "indent = 2 from the mode definition");
}

#[test]
#[allow(clippy::unnecessary_to_owned)]
fn lua_minor_mode_keymap() {
    let mut le = with_init();
    le.run("fancy-mode");
    assert!(le
        .ed
        .minor_mode_enabled(le.ed.selected_buffer_index(), "fancy"));
    le.ed.push_key(emacs_core::key::Key::ctrl('c'));
    le.ed.push_key(emacs_core::key::Key::plain('y'));
    assert_eq!(
        le.ed.lookup_key(&le.ed.pending_keys().to_vec()),
        emacs_core::keymap::Lookup::Command("say-hello".into())
    );
    le.ed.clear_pending_keys();
}

#[test]
fn undefined_command_reports_error() {
    let mut le = with_init();
    le.run("no-such-command");
    assert_eq!(le.ed.echo(), Some("no-such-command is undefined"));
}

#[test]
fn command_error_reports_message_without_location() {
    let mut le = with_init();
    // kill-region without a mark raises a Lua error inside the command
    le.run("kill-region");
    assert_eq!(
        le.ed.echo(),
        Some("The mark is not set now, so there is no region")
    );
}

#[test]
fn replay_key_flag() {
    let mut le = with_init();
    // after a command finishes with set_replay, finish_command hands the
    // key back to the loop
    le.ed.set_read_key(emacs_core::key::Key::ctrl('a'));
    le.ed.set_replay();
    let key = le
        .ed
        .finish_command(emacs_core::script::CommandOutcome::Done);
    assert_eq!(key, Some(emacs_core::key::Key::ctrl('a')));
}

#[test]
fn pending_request_survives_unknown_yield_type_check() {
    // without a script host attached, commands are no-ops (NullHost)
    let mut ed = emacs_core::editor::Editor::new(24, 80);
    let outcome = ed.call_command("anything", None).unwrap();
    assert!(matches!(outcome, emacs_core::script::CommandOutcome::Done));
    let outcome = ed
        .resume_pending(emacs_core::script::ResumeValue::String(None))
        .unwrap();
    assert!(matches!(outcome, emacs_core::script::CommandOutcome::Done));
}

#[test]
fn switch_to_buffer_finds_or_creates() {
    let mut le = with_init();
    // switch to a new buffer: prompts for the name, creates it
    le.command_reading("switch-to-buffer");
    assert!(!le.answer("notes"));
    assert_eq!(le.ed.buf().name(), "notes", "created and selected");
    // switching back to an existing buffer by name
    le.command_reading("switch-to-buffer");
    assert!(!le.answer("*scratch*"));
    assert_eq!(le.ed.buf().name(), "*scratch*", "existing buffer selected");
}
