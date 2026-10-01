//! Auto-indentation, driven through the real Lua runtime (ported from the
//! old Rust indent tests).

mod common;

use common::LuaEd;

fn rust_ed(text: &str) -> LuaEd {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert(text);
    le.run("rust-mode");
    le
}

#[test]
fn indent_after_open_brace() {
    let mut le = rust_ed("fn main() {");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("newline-and-indent");
    assert_eq!(le.text(), "fn main() {\n    ");
    assert_eq!(le.point(), 16);
}

#[test]
fn outdent_on_closing_brace() {
    let mut le = rust_ed("fn main() {\n    x();\n}");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("newline-and-indent");
    assert_eq!(le.text(), "fn main() {\n    x();\n}\n");
}

#[test]
fn tab_reindents_line() {
    let mut le = rust_ed("    x();\n");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("indent-for-tab-command");
    assert_eq!(le.text(), "x();\n");
}

#[test]
fn backspace_deletes_indent_unit() {
    let mut le = rust_ed("        x();\n");
    le.ed.buf_mut().move_to_buffer_start();
    le.ed.buf_mut().set_point(8); // first non-whitespace column
    le.run("backward-delete-char");
    assert_eq!(le.text(), "    x();\n");
}

#[test]
fn backspace_at_line_start_does_nothing() {
    let mut le = rust_ed("    x();\n");
    le.ed.buf_mut().move_to_buffer_start();
    le.run("backward-delete-char");
    assert_eq!(le.text(), "    x();\n", "nothing to delete at column 0");
}

#[test]
fn lua_end_outdents() {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert("function f()\n    print()\nend");
    le.run("lua-mode");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("newline-and-indent");
    assert_eq!(le.text(), "function f()\n    print()\nend\n");
}

#[test]
fn electric_newline_indents_normally() {
    let mut le = rust_ed("fn main() {");
    le.ed.buf_mut().move_to_buffer_end();
    le.run("electric-newline-and-maybe-indent");
    assert_eq!(le.text(), "fn main() {\n    ");
}

#[test]
fn fundamental_mode_tab_inserts_tab() {
    let mut le = LuaEd::new();
    le.ed.buf_mut().insert("x");
    le.run("indent-for-tab-command");
    assert_eq!(
        le.text(),
        "x\t",
        "no indent unit: TAB inserts a tab at point"
    );
}
