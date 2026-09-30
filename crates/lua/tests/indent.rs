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
    le.ed.refresh_syntax_current();
    le.ed.buf_mut().move_to_buffer_end();
    le.run("electric-newline-and-maybe-indent");
    assert_eq!(le.text(), "fn main() {\n    ");
}

#[test]
fn electric_newline_skips_indent_inside_string() {
    let mut le = rust_ed("fn main() {\n    let s = \"text\";\n}");
    le.ed.refresh_syntax_current();
    // point inside the string literal ("te|xt"): insert a newline there
    let line1 = le.ed.buf().rope().line_to_char(1);
    le.ed.buf_mut().set_point(line1 + 15); // 'x' of "text"
    le.run("electric-newline-and-maybe-indent");
    // the new line starts right at "xt\";" with no indentation
    assert!(le.text().contains("\nxt"));
    assert!(!le.text().contains("\n    xt"));
}

#[test]
fn electric_newline_skips_indent_inside_comment() {
    let mut le = rust_ed("fn main() {\n    // note\n}");
    le.ed.refresh_syntax_current();
    // point inside the comment text ("// no|te")
    let line1 = le.ed.buf().rope().line_to_char(1);
    le.ed.buf_mut().set_point(line1 + 9); // 't' of "note"
    le.run("electric-newline-and-maybe-indent");
    // the new line starts with "te\n}" content and no indent
    assert!(le.text().contains("\nte"));
    assert!(!le.text().contains("\n    te"));
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
