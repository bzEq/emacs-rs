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
            cycle: usize::MAX,
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

    /// Store candidates for display and, if `fill` is set, extend the input
    /// to their longest common prefix. Resets the cycle position when the
    /// candidate set changes. `fill` must be false after deletions, so the
    /// auto-fill does not re-insert what the user just deleted.
    pub fn complete_with(&mut self, candidates: Vec<String>, fill: bool) {
        if self.candidates != candidates {
            self.cycle = usize::MAX;
        }
        if candidates.is_empty() {
            self.candidates.clear();
            return;
        }
        if fill {
            let mut lcp: &str = &candidates[0];
            for c in &candidates[1..] {
                lcp = common_prefix(lcp, c);
            }
            if lcp.chars().count() > self.input.chars().count() {
                self.input = lcp.to_string();
                self.cursor = self.input.chars().count();
            }
        }
        self.candidates = candidates;
    }

    /// Cycle the input through the candidates (TAB after the common prefix
    /// is already filled). Returns false if there are fewer than two
    /// candidates.
    pub fn cycle(&mut self) -> bool {
        if self.candidates.len() < 2 {
            return false;
        }
        self.cycle = self.cycle.wrapping_add(1) % self.candidates.len();
        self.input = self.candidates[self.cycle].clone();
        self.cursor = self.input.chars().count();
        true
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

    #[test]
    fn completion_fills_lcp_then_cycles() {
        let mut mb = Minibuffer::new("M-x ".into(), None);
        mb.insert_char('d');
        mb.complete_with(vec!["delete-char".into(), "describe-key".into()], true);
        assert_eq!(mb.input, "de", "common prefix filled");
        assert_eq!(mb.candidates.len(), 2);
        // TAB again: cycle through candidates
        assert!(mb.cycle());
        assert_eq!(mb.input, "delete-char");
        assert!(mb.cycle());
        assert_eq!(mb.input, "describe-key");
        assert!(mb.cycle());
        assert_eq!(mb.input, "delete-char", "wraps around");
    }

    #[test]
    fn single_candidate_fills_completely() {
        let mut mb = Minibuffer::new("M-x ".into(), None);
        mb.insert_char('l');
        mb.insert_char('u');
        mb.complete_with(vec!["lua-mode".into()], true);
        assert_eq!(mb.input, "lua-mode");
        assert!(!mb.cycle(), "single candidate does not cycle");
    }

    #[test]
    fn cycle_resets_when_candidates_change() {
        let mut mb = Minibuffer::new("M-x ".into(), None);
        mb.complete_with(vec!["a-command".into(), "b-command".into()], true);
        assert!(mb.cycle());
        assert_eq!(mb.input, "a-command");
        // new input -> new candidate set -> cycle restarts
        mb.complete_with(
            vec!["a-command".into(), "b-command".into(), "c-command".into()],
            true,
        );
        assert!(mb.cycle());
        assert_eq!(mb.input, "a-command");
    }

    #[test]
    fn no_fill_after_deletion() {
        let mut mb = Minibuffer::new("M-x ".into(), None);
        mb.complete_with(
            vec!["describe-bindings".into(), "describe-key".into()],
            true,
        );
        assert_eq!(mb.input, "describe-", "auto-fill on insert");
        // user hits backspace: refresh candidates without re-filling
        mb.delete_backward();
        mb.complete_with(
            vec!["describe-bindings".into(), "describe-key".into()],
            false,
        );
        assert_eq!(mb.input, "describe", "deleted char stays deleted");
    }
}
