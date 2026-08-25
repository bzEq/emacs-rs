//! tree-sitter based syntax highlighting: parse a buffer, classify node
//! kinds into highlight groups, and extract per-line styled segments.

use ropey::Rope;
use tree_sitter::{Node, Parser, Tree};

use crate::buffer::Buffer;
use crate::mode::Lang;

/// Buffers larger than this (in chars) are never parsed.
pub const MAX_PARSE_CHARS: usize = 1_000_000;
/// Buffers larger than this keep their initial parse and skip re-parsing
/// on edits.
pub const MAX_REPARSE_CHARS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Keyword,
    String,
    Comment,
    Number,
    Type,
    Function,
    Constant,
}

#[derive(Debug)]
pub struct Syntax {
    pub lang: Lang,
    pub tree: Tree,
}

pub fn parse(lang: Lang, text: &str) -> Option<Syntax> {
    let mut parser = Parser::new();
    let language = match lang {
        Lang::Rust => tree_sitter::Language::new(tree_sitter_rust::LANGUAGE),
        Lang::Lua => tree_sitter::Language::new(tree_sitter_lua::LANGUAGE),
        Lang::Cpp => tree_sitter::Language::new(tree_sitter_cpp::LANGUAGE),
    };
    parser.set_language(&language).ok()?;
    let tree = parser.parse(text, None)?;
    Some(Syntax { lang, tree })
}

/// Whether a node kind is a comment or a string, per language.
pub fn kind_is_comment_or_string(lang: Lang, kind: &str) -> bool {
    match lang {
        Lang::Rust => matches!(
            kind,
            "line_comment"
                | "block_comment"
                | "string_literal"
                | "char_literal"
                | "raw_string_literal"
        ),
        Lang::Lua => matches!(kind, "comment" | "string"),
        Lang::Cpp => matches!(
            kind,
            "comment"
                | "string_literal"
                | "raw_string_literal"
                | "char_literal"
                | "system_lib_string"
                | "string_content"
        ),
    }
}

/// Whether the given char offset lies inside a comment or string node
/// (Emacs `syntax-ppss` equivalent, used by
/// electric-newline-and-maybe-indent).
pub fn point_in_comment_or_string(syntax: &Syntax, rope: &Rope, char_idx: usize) -> bool {
    let len = rope.len_chars();
    if len == 0 {
        return false;
    }
    let byte = rope.char_to_byte(char_idx.min(len));
    let mut node = syntax.tree.root_node();
    loop {
        if kind_is_comment_or_string(syntax.lang, node.kind()) {
            return true;
        }
        let mut next = None;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.start_byte() <= byte && byte < child.end_byte() {
                next = Some(child);
                break;
            }
        }
        match next {
            Some(c) => node = c,
            None => return false,
        }
    }
}

/// Highlight group for a node kind, per language.
pub fn group_for(lang: Lang, kind: &str) -> Option<Group> {
    use Group::*;
    match lang {
        Lang::Rust => match kind {
            "fn" | "let" | "if" | "else" | "match" | "impl" | "struct" | "enum" | "use" | "mod"
            | "pub" | "return" | "for" | "while" | "loop" | "in" | "as" | "const" | "static"
            | "mut" | "where" | "trait" | "async" | "await" | "crate" | "self" | "ref" | "move"
            | "unsafe" | "type" | "dyn" | "continue" | "break" | "extern" => Some(Keyword),
            "string_literal" | "char_literal" | "raw_string_literal" => Some(String),
            "line_comment" | "block_comment" => Some(Comment),
            "integer_literal" | "float_literal" => Some(Number),
            "boolean_literal" => Some(Constant),
            "type_identifier" | "primitive_type" => Some(Type),
            _ => None,
        },
        Lang::Lua => match kind {
            "return" | "local" | "if" | "then" | "else" | "elseif" | "end" | "for" | "while"
            | "do" | "repeat" | "until" | "in" | "nil" | "true" | "false" | "not" | "and"
            | "or" | "break" | "goto" | "function" => Some(Keyword),
            "string" => Some(String),
            "comment" => Some(Comment),
            "number" => Some(Number),
            _ => None,
        },
        Lang::Cpp => match kind {
            "if" | "else" | "for" | "while" | "do" | "return" | "switch" | "case" | "break"
            | "continue" | "struct" | "class" | "union" | "enum" | "namespace" | "using"
            | "typedef" | "template" | "typename" | "public" | "private" | "protected"
            | "virtual" | "static" | "const" | "constexpr" | "inline" | "new" | "delete"
            | "this" | "true" | "false" | "nullptr" | "try" | "catch" | "throw" | "sizeof"
            | "auto" | "extern" | "volatile" | "register" | "explicit" | "friend" | "operator"
            | "override" | "final" | "goto" | "default" => Some(Keyword),
            "string_literal" | "raw_string_literal" | "char_literal" | "system_lib_string" => {
                Some(String)
            }
            "comment" => Some(Comment),
            "number_literal" => Some(Number),
            "type_identifier" | "primitive_type" | "sized_type_specifier" => Some(Type),
            _ => None,
        },
    }
}

/// A highlight segment, in char columns relative to the line start.
#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub group: Group,
}

struct RawSeg {
    start: usize,
    end: usize,
    group: Group,
    depth: usize,
}

/// First `identifier`-kind node within a subtree (function names).
fn first_identifier(node: Node<'_>) -> Option<Node<'_>> {
    if node.kind() == "identifier" || node.kind() == "field_identifier" {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = first_identifier(child) {
            return Some(found);
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn collect(
    node: Node<'_>,
    lang: Lang,
    start_byte: usize,
    end_byte: usize,
    line_start_char: usize,
    buf: &Buffer,
    out: &mut Vec<RawSeg>,
    depth: usize,
) {
    let nb = node.start_byte();
    let ne = node.end_byte();
    if nb >= end_byte || ne <= start_byte {
        return;
    }

    let kind = node.kind();
    let mut push_range = |s: usize, e: usize, g: Group| {
        let s = s.clamp(nb, ne);
        let e = e.clamp(nb, ne);
        let sc = buf.rope().byte_to_char(s).saturating_sub(line_start_char);
        let ec = buf.rope().byte_to_char(e).saturating_sub(line_start_char);
        if ec > sc {
            out.push(RawSeg {
                start: sc,
                end: ec,
                group: g,
                depth,
            });
        }
    };

    if let Some(g) = group_for(lang, kind) {
        push_range(nb, ne, g);
    }

    // function names (Rust): color the name of fn items
    if lang == Lang::Rust && (kind == "function_item" || kind == "function_signature_item") {
        if let Some(name) = node.named_child(0) {
            push_range(name.start_byte(), name.end_byte(), Group::Function);
        }
    }

    // function names (C++): color the declarator's identifier
    if lang == Lang::Cpp && kind == "function_definition" {
        if let Some(decl) = node.child_by_field_name("declarator") {
            if let Some(name) = first_identifier(decl) {
                push_range(name.start_byte(), name.end_byte(), Group::Function);
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(
            child,
            lang,
            start_byte,
            end_byte,
            line_start_char,
            buf,
            out,
            depth + 1,
        );
    }
}

/// Highlight segments for one line. Deeper (more specific) nodes win over
/// their ancestors.
pub fn line_segments(syntax: &Syntax, buf: &Buffer, line_idx: usize) -> Vec<Segment> {
    let line_start_char = buf.rope().line_to_char(line_idx);
    let line_end_char = line_start_char + buf.line_len_chars(line_idx);
    if line_end_char <= line_start_char {
        return Vec::new();
    }
    let start_byte = buf.rope().char_to_byte(line_start_char);
    let end_byte = buf.rope().char_to_byte(line_end_char);

    let mut raw = Vec::new();
    collect(
        syntax.tree.root_node(),
        syntax.lang,
        start_byte,
        end_byte,
        line_start_char,
        buf,
        &mut raw,
        0,
    );
    // start asc, then innermost (deepest) first
    raw.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(b.depth.cmp(&a.depth))
            .then(b.end.cmp(&a.end))
    });
    // greedy fill: first segment covering a column wins
    let mut filled: Vec<Segment> = Vec::new();
    let mut last_end = 0usize;
    for s in raw {
        if s.start >= last_end {
            filled.push(Segment {
                start: s.start,
                end: s.end,
                group: s.group,
            });
            last_end = last_end.max(s.end);
        }
    }
    filled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::mode_for_path;

    fn buf_with(name: &str, text: &str) -> Buffer {
        let mut b = Buffer::from_reader(name, text.as_bytes()).unwrap();
        let lang = mode_for_path(name).lang.expect("lang");
        let s = parse(lang, text).expect("parse");
        b.set_syntax(Some(s));
        b
    }

    #[test]
    fn rust_keyword_and_comment() {
        let b = buf_with("t.rs", "fn main() { // hi\n}\n");
        let segs = line_segments(b.syntax().unwrap(), &b, 0);
        assert!(segs.iter().any(|s| s.group == Group::Keyword));
        assert!(segs.iter().any(|s| s.group == Group::Comment));
        // "fn" at cols 0-2
        let kw = segs.iter().find(|s| s.group == Group::Keyword).unwrap();
        assert_eq!((kw.start, kw.end), (0, 2));
    }

    #[test]
    fn rust_string() {
        let b = buf_with("t.rs", r#"let x = "hello";"#);
        let segs = line_segments(b.syntax().unwrap(), &b, 0);
        let s = segs.iter().find(|s| s.group == Group::String).unwrap();
        assert_eq!(&b.rope().to_string()[s.start..s.end], "\"hello\"");
    }

    #[test]
    fn lua_keywords() {
        let b = buf_with("t.lua", "local function f() return 42 end");
        let segs = line_segments(b.syntax().unwrap(), &b, 0);
        let kw = segs.iter().filter(|s| s.group == Group::Keyword).count();
        assert!(kw >= 3, "local/function/return/end");
        assert!(segs.iter().any(|s| s.group == Group::Number));
    }

    #[test]
    fn function_name_colored() {
        let b = buf_with("t.rs", "fn hello() {}\n");
        let segs = line_segments(b.syntax().unwrap(), &b, 0);
        assert!(
            segs.iter()
                .any(|s| s.group == Group::Function && s.start == 3 && s.end == 8),
            "hello is 3..8"
        );
    }

    #[test]
    fn cpp_keywords_strings_comments() {
        let b = buf_with("t.inc", "int main() { return 42; } // hi\n");
        let segs = line_segments(b.syntax().unwrap(), &b, 0);
        assert!(segs.iter().any(|s| s.group == Group::Keyword), "return");
        assert!(segs.iter().any(|s| s.group == Group::Comment));
        assert!(segs.iter().any(|s| s.group == Group::Number));
        assert!(
            segs.iter()
                .any(|s| s.group == Group::Type && b.rope().to_string()[s.start..s.end] == *"int"),
            "int is a primitive type"
        );
    }

    #[test]
    fn cpp_function_name_colored() {
        let b = buf_with("t.cpp", "int greet() { return 0; }\n");
        let segs = line_segments(b.syntax().unwrap(), &b, 0);
        assert!(
            segs.iter().any(|s| s.group == Group::Function && {
                let text = b.rope().to_string();
                &text[s.start..s.end] == "greet"
            }),
            "greet colored as a function"
        );
    }
}
