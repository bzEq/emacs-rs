//! Key representation and Emacs-style key sequence parsing/display.

use std::fmt;

use bitflags::bitflags;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Modifiers: u8 {
        const CONTROL = 1;
        const ALT     = 2;
        const SHIFT   = 4;
        const SUPER   = 8;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyCode {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Esc,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub mods: Modifiers,
}

impl Key {
    pub fn plain(c: char) -> Self {
        Key {
            code: KeyCode::Char(c),
            mods: Modifiers::empty(),
        }
    }

    pub fn ctrl(c: char) -> Self {
        Key {
            code: KeyCode::Char(c),
            mods: Modifiers::CONTROL,
        }
    }

    pub fn alt(c: char) -> Self {
        Key {
            code: KeyCode::Char(c),
            mods: Modifiers::ALT,
        }
    }

    #[allow(clippy::self_named_constructors)]
    pub fn key(code: KeyCode) -> Self {
        Key {
            code,
            mods: Modifiers::empty(),
        }
    }

    /// True if this key can be self-inserted when unbound.
    pub fn is_self_insertable(&self) -> bool {
        matches!(self.code, KeyCode::Char(c) if !c.is_control()
            && !self.mods.intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.contains(Modifiers::CONTROL) {
            write!(f, "C-")?;
        }
        if self.mods.contains(Modifiers::ALT) {
            write!(f, "M-")?;
        }
        if self.mods.contains(Modifiers::SUPER) {
            write!(f, "s-")?;
        }
        if self.mods.contains(Modifiers::SHIFT) {
            write!(f, "S-")?;
        }
        match self.code {
            KeyCode::Char(c) => {
                // keep e.g. C-x lowercased for control chars
                let c = if self.mods.contains(Modifiers::CONTROL) && c.is_ascii_uppercase() {
                    c.to_ascii_lowercase()
                } else {
                    c
                };
                if c == ' ' {
                    write!(f, "SPC")
                } else {
                    write!(f, "{c}")
                }
            }
            KeyCode::Enter => write!(f, "RET"),
            KeyCode::Tab => write!(f, "TAB"),
            KeyCode::Backspace => write!(f, "DEL"),
            KeyCode::Delete => write!(f, "<delete>"),
            KeyCode::Esc => write!(f, "ESC"),
            KeyCode::Left => write!(f, "<left>"),
            KeyCode::Right => write!(f, "<right>"),
            KeyCode::Up => write!(f, "<up>"),
            KeyCode::Down => write!(f, "<down>"),
            KeyCode::Home => write!(f, "<home>"),
            KeyCode::End => write!(f, "<end>"),
            KeyCode::PageUp => write!(f, "<prior>"),
            KeyCode::PageDown => write!(f, "<next>"),
            KeyCode::F(n) => write!(f, "<f{n}>"),
        }
    }
}

/// Parse an Emacs-style key sequence like `"C-x C-f"`, `"M-x"`, `"C-M-a"`.
/// Keys are separated by whitespace; special keys in angle brackets or the
/// names SPC/RET/TAB/DEL/ESC are supported.
pub fn parse_sequence(seq: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    for token in seq.split_whitespace() {
        keys.push(parse_key(token)?);
    }
    Ok(keys)
}

fn parse_key(token: &str) -> Result<Key, String> {
    let mut mods = Modifiers::empty();
    let mut rest = token;
    loop {
        if let Some(r) = rest.strip_prefix("C-") {
            mods |= Modifiers::CONTROL;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("M-") {
            mods |= Modifiers::ALT;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("S-") {
            mods |= Modifiers::SHIFT;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("A-") {
            mods |= Modifiers::ALT;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("s-") {
            mods |= Modifiers::SUPER;
            rest = r;
        } else {
            break;
        }
    }
    let code = match rest.to_ascii_uppercase().as_str() {
        "RET" | "RETURN" => KeyCode::Enter,
        "TAB" => KeyCode::Tab,
        "DEL" | "BACKSPACE" => KeyCode::Backspace,
        "ESC" => KeyCode::Esc,
        "SPC" | "SPACE" => KeyCode::Char(' '),
        _ if rest.starts_with('<') && rest.ends_with('>') => {
            let name = rest[1..rest.len() - 1].to_ascii_lowercase();
            match name.as_str() {
                "left" => KeyCode::Left,
                "right" => KeyCode::Right,
                "up" => KeyCode::Up,
                "down" => KeyCode::Down,
                "home" => KeyCode::Home,
                "end" => KeyCode::End,
                "prior" | "pageup" => KeyCode::PageUp,
                "next" | "pagedown" => KeyCode::PageDown,
                "delete" => KeyCode::Delete,
                "escape" | "esc" => KeyCode::Esc,
                "return" => KeyCode::Enter,
                "tab" => KeyCode::Tab,
                "backspace" => KeyCode::Backspace,
                _ if name.starts_with('f') => name[1..]
                    .parse()
                    .map(KeyCode::F)
                    .map_err(|_| format!("bad key: {token}"))?,
                _ => return Err(format!("unknown key: {token}")),
            }
        }
        _ => {
            let mut chars = rest.chars();
            let c = chars
                .next()
                .ok_or_else(|| format!("empty key token in {token}"))?;
            if chars.next().is_some() {
                return Err(format!("unknown key: {token}"));
            }
            KeyCode::Char(c)
        }
    };
    Ok(Key { code, mods })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(seq: &str) -> Vec<Key> {
        parse_sequence(seq).unwrap()
    }

    #[test]
    fn parse_basic() {
        assert_eq!(s("C-x C-f"), vec![Key::ctrl('x'), Key::ctrl('f')]);
        assert_eq!(s("M-x"), vec![Key::alt('x')]);
        assert_eq!(
            s("C-M-a"),
            vec![Key {
                code: KeyCode::Char('a'),
                mods: Modifiers::CONTROL | Modifiers::ALT
            }]
        );
        assert_eq!(s("RET"), vec![Key::key(KeyCode::Enter)]);
        assert_eq!(
            s("<left> <f1>"),
            vec![Key::key(KeyCode::Left), Key::key(KeyCode::F(1))]
        );
        assert_eq!(s("SPC"), vec![Key::plain(' ')]);
        assert_eq!(s("x"), vec![Key::plain('x')]);
    }

    #[test]
    fn display() {
        assert_eq!(Key::ctrl('x').to_string(), "C-x");
        assert_eq!(Key::alt('b').to_string(), "M-b");
        assert_eq!(Key::key(KeyCode::Enter).to_string(), "RET");
        assert_eq!(Key::plain(' ').to_string(), "SPC");
        assert_eq!(Key::ctrl(' ').to_string(), "C-SPC");
    }
}
