//! Minibuffer state: an input line with optional completion, plus the
//! pending-continuation state machine used by commands that need more input.

use crate::editor::Editor;
use anyhow::Result;

/// Completion: given current input, return candidate completions.
pub type CompletionFn = fn(&Editor, &str) -> Vec<String>;

#[derive(Debug)]
pub struct Minibuffer {
    pub prompt: String,
    pub input: String,
    pub cursor: usize,
    pub completion: Option<CompletionFn>,
    /// Current completion candidates (after the last Tab), for display.
    pub candidates: Vec<String>,
    pub cycle: usize,
}

impl Minibuffer {
    pub fn new(prompt: String, completion: Option<CompletionFn>) -> Self {
        Minibuffer {
            prompt,
            input: String::new(),
            cursor: 0,
            completion,
            candidates: Vec::new(),
            cycle: 0,
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += 1;
        self.candidates.clear();
    }

    pub fn insert_str(&mut self, s: &str) {
        self.input.insert_str(self.cursor, s);
        self.cursor += s.chars().count();
        self.candidates.clear();
    }

    pub fn delete_backward(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.input.remove(self.cursor);
        }
        self.candidates.clear();
    }

    pub fn delete_forward(&mut self) {
        if self.cursor < self.input.chars().count() {
            self.input.remove(self.cursor);
        }
        self.candidates.clear();
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        let len = self.input.chars().count();
        self.cursor = (self.cursor + 1).min(len);
    }

    pub fn to_start(&mut self) {
        self.cursor = 0;
    }

    pub fn to_end(&mut self) {
        self.cursor = self.input.chars().count();
    }

    /// Fill the longest common prefix of all candidates and remember the
    /// candidates for display.
    pub fn complete_with(&mut self, candidates: Vec<String>) {
        if candidates.is_empty() {
            self.candidates.clear();
            return;
        }
        let mut lcp: &str = &candidates[0];
        for c in &candidates[1..] {
            lcp = common_prefix(lcp, c);
        }
        if lcp.chars().count() > self.input.chars().count() {
            self.input = lcp.to_string();
            self.cursor = self.input.chars().count();
        }
        self.candidates = candidates;
    }
}

fn common_prefix<'a>(a: &'a str, b: &str) -> &'a str {
    let mut len = 0;
    for (ca, cb) in a.chars().zip(b.chars()) {
        if ca != cb {
            break;
        }
        len += ca.len_utf8();
    }
    &a[..len]
}

/// Completion over command names (for M-x).
pub fn complete_command_names(ed: &Editor, input: &str) -> Vec<String> {
    ed.commands().complete(input)
}

/// Completion over buffer names (for C-x b, C-x k).
pub fn complete_buffer_names(ed: &Editor, input: &str) -> Vec<String> {
    let mut names: Vec<String> = ed
        .buffers()
        .iter()
        .map(|b| b.name().to_string())
        .filter(|n| n.starts_with(input))
        .collect();
    names.sort();
    names
}

/// Deferred continuation after minibuffer input is accepted.
pub type StringContinuation = Box<dyn FnOnce(&mut Editor, String) -> Result<()>>;
pub type BoolContinuation = Box<dyn FnOnce(&mut Editor, bool) -> Result<()>>;

/// What the editor is waiting for, outside the normal command loop.
pub enum Pending {
    /// Reading a string from the minibuffer; `cont` runs on RET.
    ReadString { cont: StringContinuation },
    YesNo {
        prompt: String,
        cont: BoolContinuation,
    },
    /// describe-key: reading a key sequence; resolves when complete.
    ReadKey { keys: Vec<crate::key::Key> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lcp() {
        assert_eq!(common_prefix("foobar", "foobaz"), "fooba");
        assert_eq!(common_prefix("abc", "abd"), "ab");
        assert_eq!(common_prefix("abc", "xyz"), "");
    }

    #[test]
    fn minibuffer_editing() {
        let mut mb = Minibuffer::new("M-x ".into(), None);
        mb.insert_char('a');
        mb.insert_char('b');
        mb.move_left();
        mb.insert_char('X');
        assert_eq!(mb.input, "aXb");
        mb.delete_backward();
        assert_eq!(mb.input, "ab");
        assert_eq!(mb.cursor, 1);
    }
}
