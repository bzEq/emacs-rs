//! PTY regression tests for Lua runtime discovery: a missing or shadowed
//! `lua/` directory must produce a clear error and never silently break
//! the editor.

mod common;

use common::{write_file, Em};

#[test]
fn invalid_lua_dir_env_shows_clear_error() {
    let mut em = Em::spawn_with_env(&[("EMACS_RS_LUA_DIR", "/nonexistent/emacs-rs-lua")], &[]);
    assert!(
        em.wait_for("EMACS_RS_LUA_DIR", 5000),
        "a clear error names the misconfigured variable"
    );
    em.quit();
}

#[test]
fn valid_lua_dir_env_loads_runtime() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("lua");
    let dir_s = dir.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_env(&[("EMACS_RS_LUA_DIR", &dir_s)], &[]);
    assert!(em.wait_for("lines", 5000), "editor ready (runtime loaded)");
    let path = write_file(&em.scratch, "t.txt", "hello\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_env(&[("EMACS_RS_LUA_DIR", &dir_s)], &[&path_s]);
    assert!(em.wait_for("hello", 5000), "file opens via the env runtime");
    em.quit();
}
