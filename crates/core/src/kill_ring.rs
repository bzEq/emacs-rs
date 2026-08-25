//! The kill ring: a fixed-size ring of killed text (Emacs kill-ring).

#[derive(Debug)]
pub struct KillRing {
    entries: Vec<String>,
    /// Index into `entries` of the entry the next yank will insert.
    current: usize,
    max: usize,
}

impl Default for KillRing {
    fn default() -> Self {
        KillRing {
            entries: Vec::new(),
            current: 0,
            max: 60,
        }
    }
}

impl KillRing {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Add a kill. If `append` and the last command was also a kill, append
    /// to the current entry (Emacs accumulates consecutive kills).
    pub fn kill(&mut self, text: String, append: bool) {
        if text.is_empty() {
            return;
        }
        if append {
            if let Some(cur) = self.entries.get_mut(self.current) {
                cur.push_str(&text);
                return;
            }
        }
        self.entries.push(text);
        if self.entries.len() > self.max {
            self.entries.remove(0);
        }
        self.current = self.entries.len() - 1;
    }

    /// Text that `yank` will insert.
    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.current).map(String::as_str)
    }

    /// Rotate to the previous entry (`yank-pop`).
    pub fn pop(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        self.current = if self.current == 0 {
            self.entries.len() - 1
        } else {
            self.current - 1
        };
        self.entries.get(self.current).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_and_yank() {
        let mut kr = KillRing::new();
        kr.kill("first".into(), false);
        kr.kill("second".into(), false);
        assert_eq!(kr.current(), Some("second"));
        assert_eq!(kr.pop(), Some("first"));
        assert_eq!(kr.pop(), Some("second"), "wraps around");
    }

    #[test]
    fn append() {
        let mut kr = KillRing::new();
        kr.kill("abc".into(), false);
        kr.kill("def".into(), true);
        assert_eq!(kr.current(), Some("abcdef"));
        assert_eq!(kr.len(), 1);
    }

    #[test]
    fn empty_kill_ignored() {
        let mut kr = KillRing::new();
        kr.kill(String::new(), false);
        assert!(kr.is_empty());
    }
}
