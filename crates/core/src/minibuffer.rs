//! Minibuffer state. The input line is a real, registered `Buffer` — while
//! a read is in progress it is the editor's *current buffer* (Emacs's
//! model), with the `minibuffer-mode` major mode and its buffer-local
//! keymap. This module holds only the state around the input line:
//! completion candidates, the preview/cycle display state, the prompt.
//! Methods that edit the input take the buffer (`Editor::buffers`) as an
//! argument, since the editor owns all buffers.

use crate::buffer::{Buffer, Direction};

#[derive(Debug, Clone)]
pub struct Minibuffer {
    pub prompt: String,
    /// Id of the registered input buffer (point = the input cursor).
    pub buffer_id: usize,
    /// Whether completion is active (candidates come from the script host).
    pub completion: bool,
    /// The pre-filled input (find-file's default directory); typing `/`
    /// while the input is still exactly this replaces it (Emacs
    /// file-name-shadow).
    pub initial: Option<String>,
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
    pub fn new(
        buffer_id: usize,
        prompt: String,
        completion: bool,
        initial: Option<String>,
    ) -> Self {
        Minibuffer {
            prompt,
            buffer_id,
            completion,
            initial: initial.filter(|i| !i.is_empty()),
            candidates: Vec::new(),
            cycle: usize::MAX,
            history_index: usize::MAX,
            preview: String::new(),
        }
    }

    /// The input text.
    pub fn input(&self, buf: &Buffer) -> String {
        buf.rope().to_string()
    }

    /// The text RET accepts: the input plus the completion preview.
    pub fn accepted(&self, buf: &Buffer) -> String {
        let mut s = self.input(buf);
        s.push_str(&self.preview);
        s
    }

    /// Display column of the input cursor (chars before point).
    pub fn cursor_col(&self, buf: &Buffer) -> usize {
        buf.rope().slice(..buf.point()).chars().count()
    }

    pub fn insert_char(&mut self, buf: &mut Buffer, c: char) {
        // file-name-shadow: typing `/` over an untouched initial input
        // replaces it, so absolute paths work without manual clearing
        if c == '/'
            && self.initial.as_deref() == Some(buf.rope().to_string().as_str())
            && buf.point() == buf.len_chars()
        {
            let len = buf.len_chars();
            if len > 0 {
                let _ = buf.delete_range(0, len);
            }
            buf.insert_char('/');
            self.preview.clear();
            self.candidates.clear();
            return;
        }
        // if the typed char matches the preview, consume one preview char
        if self.preview.starts_with(c) && buf.point() == buf.len_chars() {
            buf.insert_char(c);
            self.preview = self.preview[c.len_utf8()..].to_string();
        } else {
            buf.insert_char(c);
            self.preview.clear();
        }
        self.candidates.clear();
    }

    pub fn delete_backward(&mut self, buf: &mut Buffer) {
        buf.delete_backward();
        self.preview.clear();
        self.candidates.clear();
    }

    pub fn delete_forward(&mut self, buf: &mut Buffer) {
        buf.delete_forward();
        self.preview.clear();
        self.candidates.clear();
    }

    pub fn move_left(&mut self, buf: &mut Buffer) {
        buf.move_char(Direction::Backward);
    }

    pub fn move_right(&mut self, buf: &mut Buffer) {
        buf.move_char(Direction::Forward);
    }

    pub fn to_start(&mut self, buf: &mut Buffer) {
        buf.move_to_buffer_start();
    }

    pub fn to_end(&mut self, buf: &mut Buffer) {
        buf.move_to_buffer_end();
    }

    /// Replace the whole input (history recall), point at the end.
    pub fn set_input(&mut self, buf: &mut Buffer, text: &str) {
        let len = buf.len_chars();
        if len > 0 {
            let _ = buf.delete_range(0, len);
        }
        buf.insert(text);
        buf.move_to_buffer_end();
        self.preview.clear();
        self.candidates.clear();
    }

    /// Store candidates for display and, if `fill` is set, compute the
    /// ido-style preview (the completion suffix, shown but not part of the
    /// input). Resets the cycle position when the candidate set changes.
    /// `fill` must be false after deletions, so the preview does not
    /// re-insert what the user just deleted.
    pub fn complete_with(&mut self, buf: &Buffer, candidates: Vec<String>, fill: bool) {
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
        if fill && buf.point() == buf.len_chars() {
            let input = self.input(buf);
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
    pub fn accept_preview(&mut self, buf: &mut Buffer) {
        if !self.preview.is_empty() {
            buf.insert(&self.preview);
            buf.move_to_buffer_end();
            self.preview.clear();
        }
    }

    /// Cycle the input through the candidates (TAB after the common prefix
    /// is already filled). Returns false if there are fewer than two
    /// candidates. The candidate set survives, so cycling continues.
    pub fn cycle(&mut self, buf: &mut Buffer) -> bool {
        if self.candidates.len() < 2 {
            return false;
        }
        self.cycle = self.cycle.wrapping_add(1) % self.candidates.len();
        let chosen = self.candidates[self.cycle].clone();
        let len = buf.len_chars();
        if len > 0 {
            let _ = buf.delete_range(0, len);
        }
        buf.insert(&chosen);
        buf.move_to_buffer_end();
        self.preview.clear();
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

    fn new_mb(prompt: &str, completion: bool, initial: Option<&str>) -> (Minibuffer, Buffer) {
        let mut buffer = Buffer::new("*minibuffer*");
        if let Some(i) = initial {
            buffer.insert(i);
            buffer.move_to_buffer_end();
        }
        let id = buffer.id;
        (
            Minibuffer::new(id, prompt.into(), completion, initial.map(String::from)),
            buffer,
        )
    }

    #[test]
    fn lcp() {
        assert_eq!(common_prefix("foobar", "foobaz"), "fooba");
        assert_eq!(common_prefix("abc", "abd"), "ab");
        assert_eq!(common_prefix("abc", "xyz"), "");
    }

    #[test]
    fn minibuffer_editing() {
        let (mut mb, mut buf) = new_mb("M-x ", false, None);
        mb.insert_char(&mut buf, 'a');
        mb.insert_char(&mut buf, 'b');
        mb.move_left(&mut buf);
        mb.insert_char(&mut buf, 'X');
        assert_eq!(mb.input(&buf), "aXb");
        mb.delete_backward(&mut buf);
        assert_eq!(mb.input(&buf), "ab");
        assert_eq!(buf.point(), 1);
    }

    #[test]
    fn multibyte_editing() {
        let (mut mb, mut buf) = new_mb("", false, None);
        mb.insert_char(&mut buf, '中');
        mb.insert_char(&mut buf, '文');
        mb.insert_char(&mut buf, 'x');
        assert_eq!(mb.input(&buf), "中文x");
        mb.delete_backward(&mut buf);
        assert_eq!(mb.input(&buf), "中文");
        mb.delete_backward(&mut buf);
        assert_eq!(mb.input(&buf), "中");
        mb.insert_char(&mut buf, 'y');
        assert_eq!(mb.input(&buf), "中y");
        mb.to_start(&mut buf);
        mb.delete_forward(&mut buf);
        assert_eq!(mb.input(&buf), "y");
    }

    #[test]
    fn multibyte_cursor_motion() {
        let (mut mb, mut buf) = new_mb("", false, None);
        mb.insert_char(&mut buf, '中');
        mb.insert_char(&mut buf, '文');
        mb.move_left(&mut buf);
        mb.insert_char(&mut buf, 'x');
        assert_eq!(mb.input(&buf), "中x文");
        mb.move_right(&mut buf);
        mb.move_right(&mut buf);
        mb.insert_char(&mut buf, 'y');
        assert_eq!(mb.input(&buf), "中x文y");
    }

    #[test]
    fn initial_input_and_file_name_shadow() {
        let (mut mb, mut buf) = new_mb("Find file: ", true, Some("/home/user/"));
        assert_eq!(mb.input(&buf), "/home/user/");
        assert_eq!(buf.point(), 11, "point at the end of the initial input");
        mb.move_left(&mut buf);
        assert_eq!(buf.point(), 10);
        mb.move_right(&mut buf);
        assert_eq!(buf.point(), 11);
        mb.insert_char(&mut buf, 'h');
        assert_eq!(mb.input(&buf), "/home/user/h");
        // typing `/` over the untouched initial input replaces it
        let (mut mb, mut buf) = new_mb("Find file: ", true, Some("/home/user/"));
        mb.insert_char(&mut buf, '/');
        assert_eq!(
            mb.input(&buf),
            "/",
            "file-name-shadow replaces the default dir"
        );
        mb.delete_backward(&mut buf);
        assert_eq!(mb.input(&buf), "");
    }

    #[test]
    fn completion_preview_then_cycles() {
        let (mut mb, mut buf) = new_mb("M-x ", true, None);
        mb.insert_char(&mut buf, 'd');
        mb.complete_with(
            &buf,
            vec!["delete-char".into(), "describe-key".into()],
            true,
        );
        assert_eq!(mb.input(&buf), "d", "input untouched");
        assert_eq!(mb.preview, "e", "common prefix shown as preview");
        assert_eq!(mb.accepted(&buf), "de");
        assert_eq!(mb.candidates.len(), 2);
        mb.accept_preview(&mut buf);
        assert_eq!(mb.input(&buf), "de");
        assert!(mb.cycle(&mut buf));
        assert_eq!(mb.input(&buf), "delete-char");
        assert!(mb.cycle(&mut buf));
        assert_eq!(mb.input(&buf), "describe-key");
        assert!(mb.cycle(&mut buf));
        assert_eq!(mb.input(&buf), "delete-char", "wraps around");
    }

    #[test]
    fn single_candidate_previews_full_name() {
        let (mut mb, mut buf) = new_mb("M-x ", true, None);
        mb.insert_char(&mut buf, 'l');
        mb.insert_char(&mut buf, 'u');
        mb.complete_with(&buf, vec!["lua-mode".into()], true);
        assert_eq!(mb.input(&buf), "lu");
        assert_eq!(mb.preview, "a-mode");
        assert_eq!(mb.accepted(&buf), "lua-mode");
        assert!(!mb.cycle(&mut buf), "single candidate does not cycle");
    }

    #[test]
    fn typing_consumes_matching_preview() {
        let (mut mb, mut buf) = new_mb("M-x ", true, None);
        mb.insert_char(&mut buf, 't');
        mb.complete_with(&buf, vec!["txt-mode".into()], true);
        assert_eq!(mb.preview, "xt-mode");
        for c in "xt-mode".chars() {
            mb.insert_char(&mut buf, c);
        }
        assert_eq!(mb.input(&buf), "txt-mode", "typing through the preview");
        assert_eq!(mb.preview, "");
        assert_eq!(mb.accepted(&buf), "txt-mode");
    }

    #[test]
    fn non_matching_char_drops_preview() {
        let (mut mb, mut buf) = new_mb("M-x ", true, None);
        mb.insert_char(&mut buf, 't');
        mb.complete_with(&buf, vec!["txt-mode".into()], true);
        mb.insert_char(&mut buf, 'z');
        assert_eq!(mb.input(&buf), "tz");
        assert_eq!(mb.preview, "");
    }

    #[test]
    fn cycle_resets_when_candidates_change() {
        let (mut mb, mut buf) = new_mb("M-x ", true, None);
        mb.complete_with(&buf, vec!["a-command".into(), "b-command".into()], true);
        assert!(mb.cycle(&mut buf));
        assert_eq!(mb.input(&buf), "a-command");
        mb.complete_with(
            &buf,
            vec!["a-command".into(), "b-command".into(), "c-command".into()],
            true,
        );
        assert!(mb.cycle(&mut buf));
        assert_eq!(mb.input(&buf), "a-command");
    }

    #[test]
    fn no_fill_after_deletion() {
        let (mut mb, mut buf) = new_mb("M-x ", true, None);
        for c in "describe".chars() {
            mb.insert_char(&mut buf, c);
        }
        mb.complete_with(
            &buf,
            vec!["describe-bindings".into(), "describe-key".into()],
            true,
        );
        assert_eq!(mb.preview, "-", "auto-fill preview on insert");
        mb.delete_backward(&mut buf);
        mb.complete_with(
            &buf,
            vec!["describe-bindings".into(), "describe-key".into()],
            false,
        );
        assert_eq!(mb.input(&buf), "describ", "deleted char stays deleted");
        assert_eq!(mb.preview, "", "no preview after deletion");
    }
}
