//! Simple line indentation used by `newline-and-indent` and TAB.

use crate::buffer::Buffer;
use crate::editor::Editor;

/// Char index of the first non-whitespace char of line `idx`; returns the
/// position after the trailing whitespace if the line is blank.
fn first_non_ws(buf: &Buffer, idx: usize) -> usize {
    let line_start = buf.rope().line_to_char(idx);
    let content = buf.line_len_chars(idx);
    let mut i = line_start;
    let end = line_start + content;
    while i < end {
        let c = buf.rope().char(i);
        if c != ' ' && c != '\t' {
            break;
        }
        i += 1;
    }
    i
}

fn is_blank(buf: &Buffer, idx: usize) -> bool {
    first_non_ws(buf, idx) >= buf.rope().line_to_char(idx) + buf.line_len_chars(idx)
}

/// Previous line above `idx` that is not blank, if any.
fn prev_non_blank(buf: &Buffer, idx: usize) -> Option<usize> {
    (0..idx).rev().find(|&l| !is_blank(buf, l))
}

/// Chars that continue the previous indentation level when a line ends with
/// them (rough heuristic, cc-mode style).
fn opens_indent(c: char, prev2: Option<char>) -> bool {
    match c {
        '{' | '(' | '[' => true,
        ',' | '=' => true,
        ':' => prev2 == Some(':'),
        _ => false,
    }
}

fn closes_indent(word: &str) -> bool {
    matches!(word, "end" | "else" | "elseif" | "until")
}

/// The indentation (in spaces, tabs expanded) that line `idx` should have.
pub fn compute_indent(buf: &Buffer, idx: usize, unit: usize) -> usize {
    let Some(prev) = prev_non_blank(buf, idx) else {
        return 0;
    };
    let prev_start = buf.rope().line_to_char(prev);
    let p = first_non_ws(buf, prev);
    let mut indent = 0usize;
    for c in buf.rope().slice(prev_start..p).chars() {
        indent += if c == '\t' { unit - indent % unit } else { 1 };
    }

    let prev_len = buf.line_len_chars(prev);
    let prev_slice = buf.rope().slice(prev_start..prev_start + prev_len);

    // last two non-whitespace chars of the previous line
    let prev_chars: Vec<char> = prev_slice.chars().collect();
    let mut last2 = [None, None];
    for &c in prev_chars
        .iter()
        .rev()
        .filter(|c| **c != ' ' && **c != '\t')
    {
        if last2[0].is_none() {
            last2[0] = Some(c);
        } else {
            last2[1] = Some(c);
            break;
        }
    }

    // trailing word of the previous line (`end`-style closers)
    let mut prev_word = String::new();
    for &c in prev_chars.iter().rev() {
        if c.is_ascii_alphanumeric() || c == '_' {
            prev_word.insert(0, c);
        } else {
            break;
        }
    }

    if let Some(c) = last2[0] {
        if opens_indent(c, last2[1]) {
            indent += unit;
        }
    }
    if closes_indent(&prev_word) {
        indent = indent.saturating_sub(unit);
    }

    // this line's first char / word outdents
    let start = buf.rope().line_to_char(idx);
    let len = buf.line_len_chars(idx);
    let this_slice = buf.rope().slice(start..start + len);
    if let Some(c) = this_slice.chars().find(|c| !c.is_whitespace()) {
        if matches!(c, '}' | ')' | ']') {
            indent = indent.saturating_sub(unit);
        }
        let mut word = String::new();
        for c in this_slice.chars().skip_while(|c| c.is_whitespace()) {
            if c.is_ascii_alphanumeric() || c == '_' {
                word.push(c);
            } else {
                break;
            }
        }
        if closes_indent(&word) {
            indent = indent.saturating_sub(unit);
        }
    }

    indent
}

/// Re-indent the line containing point (TAB).
pub fn indent_line(ed: &mut Editor) {
    let Some(unit) = ed.buf().mode().indent_unit else {
        return;
    };
    let buf = ed.buf_mut();
    let line = buf.line_of_point();
    let target = compute_indent(buf, line, unit);
    let line_start = buf.rope().line_to_char(line);
    let ws_end = first_non_ws(buf, line);
    let _ = buf.delete_range(line_start, ws_end);
    let line_start = ed.buf().rope().line_to_char(line);
    ed.buf_mut().set_point(line_start);
    ed.buf_mut().insert(&" ".repeat(target));
}

/// Insert a newline and indent the new line to match the context (RET in
/// programming modes).
pub fn newline_and_indent(ed: &mut Editor) {
    let indent = ed.buf().mode().indent_unit;
    ed.buf_mut().insert("\n");
    let Some(unit) = indent else {
        return;
    };
    let buf = ed.buf_mut();
    let line = buf.line_of_point();
    let target = compute_indent(buf, line, unit);
    ed.buf_mut().insert(&" ".repeat(target));
}

/// Delete one indent unit at line start, if point is inside the indentation.
/// Returns true if an indent unit was removed.
pub fn backward_delete_indent(ed: &mut Editor) -> bool {
    let Some(unit) = ed.buf().mode().indent_unit else {
        return false;
    };
    let buf = ed.buf_mut();
    let line = buf.line_of_point();
    let line_start = buf.rope().line_to_char(line);
    let point = buf.point();
    let ws: String = buf.rope().slice(line_start..point).to_string();
    if ws.is_empty() || !ws.chars().all(|c| c == ' ' || c == '\t') {
        return false;
    }
    let trailing_spaces = ws.chars().rev().take_while(|&c| c == ' ').count();
    let to_delete = if trailing_spaces > 0 {
        trailing_spaces.min(unit)
    } else {
        1 // a tab
    };
    let _ = buf.delete_range(point - to_delete, point);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;
    use crate::mode::{lua, rust};

    fn rust_ed(text: &str) -> Editor {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().set_mode(rust());
        ed.buf_mut().insert(text);
        ed
    }

    #[test]
    fn indent_after_open_brace() {
        let mut ed = rust_ed("fn main() {");
        ed.buf_mut().move_to_buffer_end();
        newline_and_indent(&mut ed);
        assert_eq!(ed.buf().rope().to_string(), "fn main() {\n    ");
        assert_eq!(ed.buf().point(), 16);
    }

    #[test]
    fn outdent_on_closing_brace() {
        let mut ed = rust_ed("fn main() {\n    x();\n}");
        ed.buf_mut().move_to_buffer_end();
        newline_and_indent(&mut ed);
        assert_eq!(ed.buf().rope().to_string(), "fn main() {\n    x();\n}\n");
    }

    #[test]
    fn tab_reindents_line() {
        let mut ed = rust_ed("    x();\n");
        ed.buf_mut().move_to_buffer_start();
        indent_line(&mut ed);
        assert_eq!(ed.buf().rope().to_string(), "x();\n");
    }

    #[test]
    fn backspace_deletes_indent_unit() {
        let mut ed = rust_ed("        x();\n");
        ed.buf_mut().move_to_buffer_start();
        let p = first_non_ws(ed.buf(), 0);
        ed.buf_mut().set_point(p);
        assert!(backward_delete_indent(&mut ed));
        assert_eq!(ed.buf().rope().to_string(), "    x();\n");
    }

    #[test]
    fn backspace_at_line_start_does_nothing() {
        let mut ed = rust_ed("    x();\n");
        ed.buf_mut().move_to_buffer_start();
        assert!(!backward_delete_indent(&mut ed));
        assert_eq!(ed.buf().rope().to_string(), "    x();\n");
    }

    #[test]
    fn lua_end_outdents() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().set_mode(lua());
        ed.buf_mut().insert("function f()\n    print()\nend");
        ed.buf_mut().move_to_buffer_end();
        newline_and_indent(&mut ed);
        assert_eq!(
            ed.buf().rope().to_string(),
            "function f()\n    print()\nend\n"
        );
    }
}
