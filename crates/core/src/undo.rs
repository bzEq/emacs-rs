//! Emacs-style undo list: entries grouped by undo boundaries.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoEntry {
    Boundary,
    /// `text` was inserted at `pos` with char length `len`; undo removes it.
    Insert { pos: usize, len: usize },
    /// `text` was deleted at `pos`; undo re-inserts it.
    Delete { pos: usize, text: String },
}

#[derive(Debug, Default)]
pub struct UndoLog {
    entries: Vec<UndoEntry>,
}

impl UndoLog {
    pub fn record_insert(&mut self, pos: usize, len: usize) {
        self.entries.push(UndoEntry::Insert { pos, len });
    }

    pub fn record_delete(&mut self, pos: usize, text: String) {
        self.entries.push(UndoEntry::Delete { pos, text });
    }

    /// Add a boundary unless the last entry already is one.
    pub fn boundary(&mut self) {
        if !matches!(self.entries.last(), Some(UndoEntry::Boundary)) {
            self.entries.push(UndoEntry::Boundary);
        }
    }

    pub fn pop_boundary(&mut self) {
        if matches!(self.entries.last(), Some(UndoEntry::Boundary)) {
            self.entries.pop();
        }
    }

    pub fn pop(&mut self) -> Option<UndoEntry> {
        self.entries.pop()
    }

    pub fn is_empty(&self) -> bool {
        !self.entries.iter().any(|e| *e != UndoEntry::Boundary)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
