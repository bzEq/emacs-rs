//! PTY regression tests for the Lua runtime: it is embedded in the binary
//! (a single `em` needs no `lua/` next to it), and `EMACS_RS_LUA_DIR` is a
//! validated override for developing the runtime.

mod common;

use common::{scratch_dir, write_file, Em};

#[test]
fn single_binary_without_lua_dir_works() {
    // copy the binary into a bare directory with no lua/ anywhere near it
    let scratch = scratch_dir();
    let bin = scratch.join("em");
    std::fs::copy(common::em_binary(), &bin).unwrap();
    let path = write_file(&scratch, "t.txt", "hello\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_bin(scratch, &bin, &[], &[&path_s]);
    assert!(
        em.wait_for("hello", 5000),
        "the embedded runtime opens files with no lua/ directory present"
    );
    em.quit();
}

#[test]
fn invalid_lua_dir_env_is_fatal() {
    let mut em = Em::spawn_with_env(&[("EMACS_RS_LUA_DIR", "/nonexistent/emacs-rs-lua")], &[]);
    assert!(
        em.wait_for("EMACS_RS_LUA_DIR", 5000),
        "the fatal error names the misconfigured variable"
    );
    let status = em.exit_status().unwrap();
    assert_eq!(status.code(), Some(1), "fatal runtime error exits 1");
}

#[test]
fn missing_runtime_module_is_fatal() {
    // a runtime directory that is missing one required module
    let em = Em::spawn();
    let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("lua");
    let dst = em.scratch.join("partial-lua");
    std::fs::create_dir_all(&dst).unwrap();
    for f in std::fs::read_dir(&src).unwrap() {
        let f = f.unwrap();
        let name = f.file_name().into_string().unwrap();
        if name != "dired.lua" {
            std::fs::copy(f.path(), dst.join(name)).unwrap();
        }
    }
    let dst_s = dst.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_env(&[("EMACS_RS_LUA_DIR", &dst_s)], &[]);
    assert!(
        em.wait_for("required Lua runtime module is missing", 5000),
        "the fatal error names the missing module"
    );
    let status = em.exit_status().unwrap();
    assert_eq!(status.code(), Some(1));
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
