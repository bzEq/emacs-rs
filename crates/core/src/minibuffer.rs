//! Minibuffer state: the input is a real rope `Buffer` (Emacs's model —
//! while the minibuffer is active, the input is the current buffer, so
//! global keybindings and editing commands operate on it). Completion
//! candidates come from the scripting host (Lua); this module also holds
//! the preview/cycle display state.

use crate::buffer::Buffer;

#[derive(Debug, Clone)]
pub struct Minibuffer {
    pub prompt: String,
    /// The input line, as a buffer (point = the input cursor).
    buffer: Buffer,
    /// Whether completion is active (candidates come from the script host).
    pub completion: bool,
    /// The pre-filled input (find-file's default directory); typing `/`
    /// while the input is still exactly this replaces it (Emacs
    /// file-name-shadow).
    pub initial: String,
    /// Current completion candidates (after the last Tab), for display.
    pub candidates: Vec<String>,
    pub cycle: usize,
    /// Position in the editor-wide input history; `== history.len()` means
    /// the fresh entry past the end (C-n/C-p walk this index).
    pub history_index: usize,
    /// Auto-completion preview: the part of the completed name that is
    /// shown but not yet part of the input (ido-style). Typing a matching
    /// character consumes it; RET accepts it.
    pub preview: String,
}

impl Minibuffer {
    pub fn new(prompt: String, completion: bool, initial: Option<String>) -> Self {
        let initial = initial.filter(|i| !i.is_empty()).unwrap_or_default();
        let mut buffer = Buffer::new("*minibuffer*");
        if !initial.is_empty() {
            buffer.insert(&initial);
            buffer.move_to_buffer_end();
        }
        Minibuffer {
            prompt,
            buffer,
            completion,
            initial,
            candidates: Vec::new(),
            cycle: usize::MAX,
            history_index: usize::MAX,
            preview: String::new(),
        }
    }

    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    pub fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffer
    }

    /// The input text.
    pub fn input(&self) -> String {
        self.buffer.rope().to_string()
    }

    /// The input text that RET accepts: the input plus the completion
    /// preview.
    pub fn accepted(&self) -> String {
        let mut s = self.input();
        s.push_str(&self.preview);
        s
    }

    /// Display column of the input cursor (chars before point).
    pub fn cursor_col(&self) -> usize {
        self.buffer
            .rope()
            .slice(..self.buffer.point())
            .chars()
            .count()
    }

    pub fn insert_char(&mut self, c: char) {
        // file-name-shadow: typing `/` over an untouched initial input
        // replaces it, so absolute paths work without manual clearing
        if c == '/'
            && !self.initial.is_empty()
            && self.buffer.rope().to_string() == self.initial
            && self.buffer.point() == self.initial.len()
        {
            self.buffer.delete_range(0, self.buffer.len_chars());
            self.buffer.insert_char('/');
            self.preview.clear();
            self.candidates.clear();
            return;
        }
        // if the typed char matches the preview, consume one preview char
        if self.preview.starts_with(c) && self.buffer.point() == self.buffer.len_chars() {
            self.buffer.insert_char(c);
            self.preview = self.preview[c.len_utf8()..].to_string();
        } else {
            self.buffer.insert_char(c);
            self.preview.clear();
        }
        self.candidates.clear();
    }

    pub fn delete_backward(&mut self) {
        self.buffer.delete_backward();
        self.preview.clear();
        self.candidates.clear();
    }

    pub fn delete_forward(&mut self) {
        self.buffer.delete_forward();
        self.preview.clear();
        self.candidates.clear();
    }

    /// Replace the whole input (history recall), point at the end.
    pub fn set_input(&mut self, text: &str) {
        let len = self.buffer.len_chars();
        if len > 0 {
            let _ = self.buffer.delete_range(0, len);
        }
        self.buffer.insert(text);
        self.buffer.move_to_buffer_end();
        self.preview.clear();
        self.candidates.clear();
    }

    /// Store candidates for display and, if `fill` is set, compute the
    /// ido-style preview (the completion suffix, shown but not part of the
    /// input). Resets the cycle position when the candidate set changes.
    /// `fill` must be false after deletions, so the preview does not
    /// re-insert what the user just deleted.
    pub fn complete_with(&mut self, candidates: Vec<String>, fill: bool) {
        if self.candidates != candidates {
            self.cycle = usize::MAX;
        }
        if !fill {
            self.preview.clear();
        }
        if candidates.is_empty() {
            self.preview.clear();
            self.candidates.clear();
            return;
        }
        if fill && self.buffer.point() == self.buffer.len_chars() {
            let input = self.input();
            // preview = LCP of the candidates (full name if unique)
            let mut lcp: &str = &candidates[0];
            for c in &candidates[1..] {
                lcp = common_prefix(lcp, c);
            }
            if lcp.len() > input.len() && lcp.starts_with(&input) {
                self.preview = lcp[input.len()..].to_string();
            }
        }
        self.candidates = candidates;
    }

    /// Fold the preview into the input (TAB or RET).
    pub fn accept_preview(&mut self) {
        if !self.preview.is_empty() {
            self.buffer.insert(&self.preview);
            self.buffer.move_to_buffer_end();
            self.preview.clear();
        }
    }

    /// Cycle the input through the candidates (TAB after the common prefix
    /// is already filled). Returns false if there are fewer than two
    /// candidates.
    pub fn cycle(&mut self) -> bool {
        if self.candidates.len() < 2 {
            return false;
        }
        self.cycle = self.cycle.wrapping_add(1) % self.candidates.len();
        self.set_input(&self.candidates[self.cycle].clone());
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
        let mut mb = Minibuffer::new("M-x ".into(), false, None);
        mb.insert_char('a');
        mb.insert_char('b');
        mb.buffer_mut().move_char(crate::buffer::Direction::Backward);
        mb.insert_char('X');
        assert_eq!(mb.input(), "aXb");
        mb.delete_backward();
        assert_eq!(mb.input(), "ab");
        assert_eq!(mb.buffer().point(), 1);
    }

    #[test]
    fn multibyte_editing() {
        let mut mb = Minibuffer::new(String::new(), false, None);
        mb.insert_char('中');
        mb.insert_char('文');
        mb.insert_char('x');
        assert_eq!(mb.input(), "中文x");
        // backspace removes the whole 'x', then whole chars
        mb.delete_backward();
        assert_eq!(mb.input(), "中文");
        mb.delete_backward();
        assert_eq!(mb.input(), "中");
        mb.insert_char('y');
        assert_eq!(mb.input(), "中y");
        // delete-forward at the start removes the whole first char
        mb.buffer_mut().move_to_buffer_start();
        mb.delete_forward();
        assert_eq!(mb.input(), "y");
    }

    #[test]
    fn multibyte_cursor_motion() {
        let mut mb = Minibuffer::new(String::new(), false, None);
        mb.insert_char('中');
        mb.insert_char('文');
        mb.buffer_mut().move_char(crate::buffer::Direction::Backward);
        mb.insert_char('x');
        assert_eq!(mb.input(), "中x文");
        mb.buffer_mut().move_char(crate::buffer::Direction::Forward);
        mb.buffer_mut().move_char(crate::buffer::Direction::Forward);
        mb.insert_char('y');
        assert_eq!(mb.input(), "中x文y");
    }

    #[test]
    fn initial_input_and_file_name_shadow() {
        let mut mb = Minibuffer::new("Find file: ".into(), true, Some("/home/user/".into()));
        assert_eq!(mb.input(), "/home/user/");
        assert_eq!(mb.buffer().point(), 11, "point at the end of the initial input");
        // motion over the pre-filled input
        mb.buffer_mut().move_char(crate::buffer::Direction::Backward);
        assert_eq!(mb.buffer().point(), 10);
        mb.buffer_mut().move_char(crate::buffer::Direction::Forward);
        assert_eq!(mb.buffer().point(), 11);
        // typing a relative name appends
        mb.insert_char('h');
        assert_eq!(mb.input(), "/home/user/h");
        // but typing `/` over the untouched initial input replaces it
        let mut mb = Minibuffer::new("Find file: ".into(), true, Some("/home/user/".into()));
        mb.insert_char('/');
        assert_eq!(mb.input(), "/", "file-name-shadow replaces the default dir");
        // backspace/delete now have text to work on
        mb.delete_backward();
        assert_eq!(mb.input(), "");
    }

    #[test]
    fn completion_preview_then_cycles() {
        let mut mb = Minibuffer::new("M-x ".into(), true, None);
        mb.insert_char('d');
        mb.complete_with(vec!["delete-char".into(), "describe-key".into()], true);
        assert_eq!(mb.input(), "d", "input untouched");
        assert_eq!(mb.preview, "e", "common prefix shown as preview");
        assert_eq!(mb.accepted(), "de");
        assert_eq!(mb.candidates.len(), 2);
        // TAB: fold the preview into the input
        mb.accept_preview();
        assert_eq!(mb.input(), "de");
        assert!(mb.cycle());
        assert_eq!(mb.input(), "delete-char");
        assert!(mb.cycle());
        assert_eq!(mb.input(), "describe-key");
        assert!(mb.cycle());
        assert_eq!(mb.input(), "delete-char", "wraps around");
    }

    #[test]
    fn single_candidate_previews_full_name() {
        let mut mb = Minibuffer::new("M-x ".into(), true, None);
        mb.insert_char('l');
        mb.insert_char('u');
        mb.complete_with(vec!["lua-mode".into()], true);
        assert_eq!(mb.input(), "lu");
        assert_eq!(mb.preview, "a-mode");
        assert_eq!(mb.accepted(), "lua-mode");
        assert!(!mb.cycle(), "single candidate does not cycle");
    }

    #[test]
    fn typing_consumes_matching_preview() {
        let mut mb = Minibuffer::new("M-x ".into(), true, None);
        mb.insert_char('t');
        mb.complete_with(vec!["txt-mode".into()], true);
        assert_eq!(mb.preview, "xt-mode");
        for c in "xt-mode".chars() {
            mb.insert_char(c);
        }
        assert_eq!(mb.input(), "txt-mode", "typing through the preview");
        assert_eq!(mb.preview, "");
        assert_eq!(mb.accepted(), "txt-mode");
    }

    #[test]
    fn non_matching_char_drops_preview() {
        let mut mb = Minibuffer::new("M-x ".into(), true, None);
        mb.insert_char('t');
        mb.complete_with(vec!["txt-mode".into()], true);
        mb.insert_char('z');
        assert_eq!(mb.input(), "tz");
        assert_eq!(mb.preview, "");
    }

    #[test]
    fn cycle_resets_when_candidates_change() {
        let mut mb = Minibuffer::new("M-x ".into(), true, None);
        mb.complete_with(vec!["a-command".into(), "b-command".into()], true);
        assert!(mb.cycle());
        assert_eq!(mb.input(), "a-command");
        // new input -> new candidate set -> cycle restarts
        mb.complete_with(
            vec!["a-command".into(), "b-command".into(), "c-command".into()],
            true,
        );
        assert!(mb.cycle());
        assert_eq!(mb.input(), "a-command");
    }

    #[test]
    fn no_fill_after_deletion() {
        let mut mb = Minibuffer::new("M-x ".into(), true, None);
        // user typed "describe" and the preview showed the LCP suffix
        for c in "describe".chars() {
            mb.insert_char(c);
        }
        mb.complete_with(
            vec!["describe-bindings".into(), "describe-key".into()],
            true,
        );
        assert_eq!(mb.preview, "-", "auto-fill preview on insert");
        // user hits backspace: refresh candidates without re-filling
        mb.delete_backward();
        mb.complete_with(
            vec!["describe-bindings".into(), "describe-key".into()],
            false,
        );
        assert_eq!(mb.input(), "describ", "deleted char stays deleted");
        assert_eq!(mb.preview, "", "no preview after deletion");
    }
}
