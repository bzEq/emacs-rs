//! Emacs-style sparse keymap: keys map to commands or sub-keymaps (prefixes).

use std::collections::HashMap;

use crate::key::Key;

#[derive(Debug, Clone, Default)]
pub struct Keymap {
    bindings: HashMap<Key, Action>,
}

#[derive(Debug, Clone)]
pub enum Action {
    Command(String),
    Prefix(Keymap),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    Command(String),
    /// The sequence so far is a valid prefix; more keys expected.
    Prefix,
    Unbound,
}

impl Keymap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Bind a full key sequence to a command, creating prefix keymaps as
    /// needed. Rebinding a key that is already a prefix replaces the whole
    /// subtree (Emacs silently replaces bindings). Passing an empty
    /// sequence is a no-op.
    pub fn bind_sequence(&mut self, seq: &[Key], command: impl Into<String>) {
        match seq {
            [] => {}
            [key] => {
                self.bindings.insert(*key, Action::Command(command.into()));
            }
            [key, rest @ ..] => {
                if !matches!(self.bindings.get(key), Some(Action::Prefix(_))) {
                    // the key was unbound or a command and becomes a prefix;
                    // a previous command binding is dropped (Emacs replaces
                    // bindings silently)
                    self.bindings.insert(*key, Action::Prefix(Keymap::new()));
                }
                match self.bindings.get_mut(key) {
                    Some(Action::Prefix(p)) => p.bind_sequence(rest, command),
                    _ => unreachable!(),
                }
            }
        }
    }

    pub fn bind(&mut self, key: Key, command: impl Into<String>) {
        self.bindings.insert(key, Action::Command(command.into()));
    }

    /// Look up a full or partial key sequence (Emacs keymap-lookup semantics).
    pub fn lookup(&self, seq: &[Key]) -> Lookup {
        match seq {
            [] => Lookup::Unbound,
            [key] => match self.bindings.get(key) {
                Some(Action::Command(name)) => Lookup::Command(name.clone()),
                Some(Action::Prefix(_)) => Lookup::Prefix,
                None => Lookup::Unbound,
            },
            [key, rest @ ..] => match self.bindings.get(key) {
                Some(Action::Prefix(p)) => p.lookup(rest),
                Some(Action::Command(_)) | None => Lookup::Unbound,
            },
        }
    }

    /// All bindings flattened to (sequence, command) pairs, for
    /// describe-bindings.
    pub fn flatten(&self) -> Vec<(Vec<Key>, String)> {
        let mut out = Vec::new();
        self.flatten_into(&mut Vec::new(), &mut out);
        out.sort_by(|a, b| {
            let sa: String = a.0.iter().map(|k| k.to_string()).collect();
            let sb: String = b.0.iter().map(|k| k.to_string()).collect();
            sa.cmp(&sb)
        });
        out
    }

    fn flatten_into(&self, prefix: &mut Vec<Key>, out: &mut Vec<(Vec<Key>, String)>) {
        let mut entries: Vec<_> = self.bindings.iter().collect();
        entries.sort_by_key(|(k, _)| k.to_string());
        for (key, action) in entries {
            prefix.push(*key);
            match action {
                Action::Command(name) => out.push((prefix.clone(), name.clone())),
                Action::Prefix(child) => child.flatten_into(prefix, out),
            }
            prefix.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{parse_sequence, Key};

    #[test]
    fn lookup_and_prefix() {
        let mut km = Keymap::new();
        km.bind_sequence(&parse_sequence("C-x C-f").unwrap(), "find-file");
        km.bind_sequence(&parse_sequence("C-x C-s").unwrap(), "save-buffer");
        km.bind(Key::ctrl('a'), "beginning-of-line");

        assert_eq!(km.lookup(&parse_sequence("C-x").unwrap()), Lookup::Prefix);
        assert_eq!(
            km.lookup(&parse_sequence("C-x C-f").unwrap()),
            Lookup::Command("find-file".into())
        );
        assert_eq!(
            km.lookup(&parse_sequence("C-x z").unwrap()),
            Lookup::Unbound
        );
        assert_eq!(
            km.lookup(&parse_sequence("C-a").unwrap()),
            Lookup::Command("beginning-of-line".into())
        );
        assert_eq!(km.lookup(&parse_sequence("C-b").unwrap()), Lookup::Unbound);
    }

    #[test]
    fn flatten() {
        let mut km = Keymap::new();
        km.bind_sequence(&parse_sequence("C-x C-f").unwrap(), "find-file");
        km.bind_sequence(&parse_sequence("C-a").unwrap(), "beginning-of-line");
        let flat = km.flatten();
        assert_eq!(flat.len(), 2);
        assert!(flat.contains(&(parse_sequence("C-x C-f").unwrap(), "find-file".into())));
    }

    #[test]
    fn rebinding_prefix_replaces_subtree() {
        let mut km = Keymap::new();
        km.bind_sequence(&parse_sequence("C-x C-f").unwrap(), "find-file");
        km.bind(Key::ctrl('x'), "other");
        assert_eq!(
            km.lookup(&parse_sequence("C-x").unwrap()),
            Lookup::Command("other".into())
        );
        assert_eq!(
            km.lookup(&parse_sequence("C-x C-f").unwrap()),
            Lookup::Unbound,
            "the replaced prefix subtree is gone"
        );
    }

    #[test]
    fn rebinding_command_as_prefix() {
        let mut km = Keymap::new();
        km.bind(Key::ctrl('x'), "other");
        km.bind_sequence(&parse_sequence("C-x C-f").unwrap(), "find-file");
        assert_eq!(
            km.lookup(&parse_sequence("C-x C-f").unwrap()),
            Lookup::Command("find-file".into())
        );
    }
}
