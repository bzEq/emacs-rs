//! Startup, editing commands, undo, and the kill ring, driven through the
//! real Lua runtime.

mod common;

use common::LuaEd;

#[test]
fn startup_opens_file() {
    let dir = std::env::temp_dir().join(format!("em-lua-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hello.txt");
    std::fs::write(&path, "hello world\n").unwrap();
    let path_s = path.display().to_string();

    let mut le = LuaEd::new();
    le.startup(Some(&path_s));
    assert_eq!(le.ed.buf().name(), "hello.txt");
    assert_eq!(le.text(), "hello world\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runtime_loads_from_a_directory() {
    // the EMACS_RS_LUA_DIR override path: the same runtime modules load
    // from the repo's lua/ directory
    let mut ed = emacs_core::editor::Editor::new(24, 80);
    let host = emacs_lua::LuaHost::new().expect("LuaJIT host");
    ed.attach_script(Box::new(host));
    ed.load_runtime_dir(&common::lua_dir())
        .expect("runtime loads from dir");
    let outcome = ed.call_command("forward-char", None).unwrap();
    assert!(matches!(outcome, emacs_core::script::CommandOutcome::Done));
}

#[test]
fn missing_module_in_dir_is_fatal() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let partial = std::env::temp_dir().join(format!("em-lua-partial-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&partial).unwrap();
    for f in std::fs::read_dir(common::lua_dir()).unwrap() {
        let f = f.unwrap();
        let name = f.file_name().into_string().unwrap();
        if name != "dired.lua" {
            std::fs::copy(f.path(), partial.join(name)).unwrap();
        }
    }
    let mut ed = emacs_core::editor::Editor::new(24, 80);
    let host = emacs_lua::LuaHost::new().expect("LuaJIT host");
    ed.attach_script(Box::new(host));
    let err = ed.load_runtime_dir(&partial).unwrap_err();
    assert!(
        err.to_string().contains("dired.lua"),
        "missing module named: {err}"
    );
    let _ = std::fs::remove_dir_all(&partial);
}
