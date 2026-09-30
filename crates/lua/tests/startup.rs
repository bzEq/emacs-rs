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
