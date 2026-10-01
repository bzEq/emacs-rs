//! PTY regression tests: auto-indentation and major modes.

mod common;

use common::{write_file, Em};

#[test]
fn ret_auto_indents_after_brace() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.rs", "fn main() {");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("fn main() {", 5000));
    // move to buffer end, RET
    em.keys(b"\x1b>"); // M->
    em.keys(b"\r");
    // indentation is whitespace (invisible on a blank screen): assert it
    // via the cursor position — L2 C4 means point sits after 4 spaces
    assert!(em.wait_for("L2 C4", 3000), "new line indented by 4");
    em.type_str("x");
    assert!(em.wait_for("    x", 3000), "x lands after the indent");
    em.keys(b"\r");
    em.type_str("}");
    em.keys(b"\t"); // TAB reindents the closing brace
    em.keys(b"\x18\x13"); // C-x C-s
    assert!(em.wait_for("Wrote", 5000));
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(saved, "fn main() {\n    x\n}", "tab dedented the brace");
    em.quit();
}

#[test]
fn mx_rust_mode_switches_mode() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "fn main() {}\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("fn main()", 5000));
    assert!(em.wait_for("(fundamental-mode)", 5000));
    em.m_x("rust-mode");
    assert!(em.wait_for("(rust-mode)", 3000), "M-x rust-mode switches");
    em.quit();
}

#[test]
fn cj_indents_normally() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.rs", "fn main() {");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("fn main() {", 5000));
    em.keys(b"\x1b>"); // M->
    em.keys(b"\x0a"); // C-j
    assert!(em.wait_for("L2 C4", 3000), "C-j indents the new line");
    em.quit();
}

#[test]
fn inc_file_opens_in_cpp_mode() {
    let em = Em::spawn();
    let path = write_file(
        &em.scratch,
        "config.inc",
        "class Widget {\npublic:\n    int size() { return 42; }\n};\n",
    );
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("Widget", 5000));
    assert!(em.wait_for("(cpp-mode)", 5000), "modeline shows cpp-mode");
    em.quit();
}
