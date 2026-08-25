//! emacs-rs: an Emacs-like editor with a rope-backed buffer.
//!
//! M1: command system, keymap with prefix keys, kill ring, undo, minibuffer
//! (M-x, C-x C-f, ...), buffer switching, and LuaJIT scripting (init.lua).

use std::io::{self, Stdout};
use std::panic;
use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use emacs_core::editor::Editor;
use emacs_core::key::{Key, KeyCode, Modifiers};
use emacs_core::keymap::Lookup;
use emacs_core::minibuffer::Pending;
use emacs_lua::LuaHost;
use emacs_ui::render;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let file_arg: Option<String> = args.next();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let size = terminal.size()?;

    let hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        hook(info);
    }));

    let mut ed = Editor::new(size.height.saturating_sub(1) as usize, size.width as usize);

    // LuaJIT scripting engine + init.lua
    match LuaHost::new() {
        Ok(host) => {
            ed.attach_script(Box::new(host));
            if let Some(init) = init_file() {
                if init.exists() {
                    if let Err(e) = ed.load_script(&init) {
                        ed.error(format!("error loading init.lua: {e}"));
                    }
                }
            }
        }
        Err(e) => eprintln!("LuaJIT unavailable: {e}"),
    }

    if let Some(path) = file_arg {
        match emacs_core::buffer::Buffer::load_file(&path) {
            Ok(buf) => {
                let id = buf.id;
                ed.add_buffer(buf);
                ed.set_selected_buffer(id);
            }
            Err(e) => ed.error(format!("cannot open {path}: {e}")),
        }
    }

    let result = run(&mut ed, &mut terminal);

    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    result
}

fn init_file() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(dir.join("emacs-rs").join("init.lua"))
}

fn run(ed: &mut Editor, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    loop {
        ed.scroll_current_view();
        terminal.draw(|f| {
            if let Some((x, y)) = render(f, ed) {
                f.set_cursor_position(ratatui::layout::Position::new(x, y));
            }
        })?;
        if ed.quit() {
            return Ok(());
        }
        match event::read()? {
            Event::Key(k) if k.kind == KeyEventKind::Press => {
                if let Some(key) = to_key(&k) {
                    if let Err(e) = handle_key(ed, key) {
                        ed.error(e.to_string());
                    }
                }
            }
            Event::Resize(w, h) => {
                ed.set_window_size(h.saturating_sub(1) as usize, w as usize);
            }
            _ => {}
        }
    }
}

/// Convert a crossterm key event to an emacs-rs key.
fn to_key(ke: &crossterm::event::KeyEvent) -> Option<Key> {
    let mut mods = Modifiers::empty();
    if ke.modifiers.contains(KeyModifiers::CONTROL) {
        mods |= Modifiers::CONTROL;
    }
    if ke.modifiers.contains(KeyModifiers::ALT) {
        mods |= Modifiers::ALT;
    }
    if ke.modifiers.contains(KeyModifiers::SHIFT) {
        mods |= Modifiers::SHIFT;
    }
    if ke.modifiers.contains(KeyModifiers::SUPER) {
        mods |= Modifiers::SUPER;
    }
    let code = match ke.code {
        crossterm::event::KeyCode::Char('\0') => KeyCode::Char(' '), // C-SPC / C-@
        // The byte 0x1F is what terminals send for C-/ and C-_; crossterm
        // reports it as C-7. Map it to C-_ so the undo binding works.
        crossterm::event::KeyCode::Char('7') if ke.modifiers.contains(KeyModifiers::CONTROL) => {
            KeyCode::Char('_')
        }
        crossterm::event::KeyCode::Char(c) => KeyCode::Char(c),
        crossterm::event::KeyCode::Enter => KeyCode::Enter,
        crossterm::event::KeyCode::Tab => KeyCode::Tab,
        crossterm::event::KeyCode::Backspace => KeyCode::Backspace,
        crossterm::event::KeyCode::Delete => KeyCode::Delete,
        crossterm::event::KeyCode::Esc => KeyCode::Esc,
        crossterm::event::KeyCode::Left => KeyCode::Left,
        crossterm::event::KeyCode::Right => KeyCode::Right,
        crossterm::event::KeyCode::Up => KeyCode::Up,
        crossterm::event::KeyCode::Down => KeyCode::Down,
        crossterm::event::KeyCode::Home => KeyCode::Home,
        crossterm::event::KeyCode::End => KeyCode::End,
        crossterm::event::KeyCode::PageUp => KeyCode::PageUp,
        crossterm::event::KeyCode::PageDown => KeyCode::PageDown,
        crossterm::event::KeyCode::F(n) => KeyCode::F(n),
        _ => return None,
    };
    Some(Key { code, mods })
}

/// One key press into the current input state (minibuffer, pending prompt,
/// or the normal keymap).
fn handle_key(ed: &mut Editor, key: Key) -> Result<()> {
    let key = translate_after_ctrl_x(ed, key);
    // Esc acts as a Meta prefix (ESC x == M-x).
    if key.code == KeyCode::Esc && key.mods.is_empty() {
        if ed.esc_prefix() {
            ed.set_esc_prefix(false);
        } else {
            ed.set_esc_prefix(true);
        }
        return Ok(());
    }
    if ed.esc_prefix() {
        ed.set_esc_prefix(false);
        if key.mods.is_empty() {
            let mut k = key;
            k.mods |= Modifiers::ALT;
            return dispatch(ed, k);
        }
        return Ok(());
    }
    dispatch(ed, key)
}

/// Terminals can't send Ctrl+<digit> distinctly (Ctrl+2 is byte 0x00, Ctrl+3
/// is ESC, ...), so after a C-x prefix we translate the raw bytes into the
/// plain digit keys the keymap expects. This is what makes C-x 2 / C-x 3
/// work on a terminal.
fn translate_after_ctrl_x(ed: &Editor, key: Key) -> Key {
    if ed.pending_keys().len() == 1 && ed.pending_keys()[0] == Key::ctrl('x') {
        let m = key.mods;
        match key.code {
            KeyCode::Char(' ') if m.contains(Modifiers::CONTROL) => Key::plain('2'),
            KeyCode::Esc => Key::plain('3'),
            KeyCode::Char('\\') if m.contains(Modifiers::CONTROL) => Key::plain('4'),
            KeyCode::Char(']') if m.contains(Modifiers::CONTROL) => Key::plain('5'),
            KeyCode::Char('^') if m.contains(Modifiers::CONTROL) => Key::plain('6'),
            KeyCode::Char('_') if m.contains(Modifiers::CONTROL) => Key::plain('7'),
            KeyCode::Backspace => Key::plain('8'),
            _ => key,
        }
    } else {
        key
    }
}

fn dispatch(ed: &mut Editor, key: Key) -> Result<()> {
    if ed.isearch_active() {
        match emacs_core::isearch::handle_key(ed, &key)? {
            emacs_core::isearch::ISearchResult::Consumed => return Ok(()),
            emacs_core::isearch::ISearchResult::Exit { replay: Some(k) } => {
                return dispatch(ed, k);
            }
            emacs_core::isearch::ISearchResult::Exit { replay: None } => return Ok(()),
        }
    }
    if ed.minibuffer().is_some() {
        return minibuffer_key(ed, key);
    }
    if ed.pending().is_some() {
        return pending_key(ed, key);
    }

    ed.clear_echo();
    ed.push_key(key);
    let seq = ed.pending_keys().to_vec();
    let name = match ed.keymap().lookup(&seq) {
        Lookup::Command(name) => Some(name.to_string()),
        Lookup::Prefix => return Ok(()),
        Lookup::Unbound => {
            if ed.pending_keys().len() == 1 && key.is_self_insertable() {
                Some("self-insert-command".to_string())
            } else {
                let seqs: Vec<String> = ed.pending_keys().iter().map(|k| k.to_string()).collect();
                ed.error(format!("{} is undefined", seqs.join(" ")));
                ed.clear_pending_keys();
                return Ok(());
            }
        }
    };
    let name = name.unwrap();
    ed.clear_pending_keys();
    if name == "self-insert-command" {
        if let KeyCode::Char(c) = key.code {
            ed.set_self_insert_char(Some(c));
        }
    }
    if let Err(e) = ed.invoke_command(&name) {
        ed.error(e.to_string());
    }
    Ok(())
}

/// Keys while the minibuffer is reading input.
fn minibuffer_key(ed: &mut Editor, key: Key) -> Result<()> {
    use KeyCode::*;
    let m = key.mods;
    match key.code {
        Char(c) if m.contains(Modifiers::CONTROL) => match c {
            'g' => {
                ed.abort_pending();
                ed.message("Quit");
            }
            'a' => {
                if let Some(mb) = ed.minibuffer_mut() {
                    mb.to_start();
                }
            }
            'e' => {
                if let Some(mb) = ed.minibuffer_mut() {
                    mb.to_end();
                }
            }
            'f' => {
                if let Some(mb) = ed.minibuffer_mut() {
                    mb.move_right();
                }
            }
            'b' => {
                if let Some(mb) = ed.minibuffer_mut() {
                    mb.move_left();
                }
            }
            'd' => {
                if let Some(mb) = ed.minibuffer_mut() {
                    mb.delete_forward();
                }
            }
            'k' => {
                if let Some(mb) = ed.minibuffer_mut() {
                    mb.input.truncate(mb.cursor);
                    mb.candidates.clear();
                }
            }
            _ => {}
        },
        Char(c) if !m.contains(Modifiers::ALT) && !m.contains(Modifiers::SUPER) => {
            if let Some(mb) = ed.minibuffer_mut() {
                mb.insert_char(c);
            }
        }
        Enter => {
            let input = ed
                .minibuffer()
                .map(|mb| mb.input.clone())
                .unwrap_or_default();
            ed.finish_read_string(input)?;
        }
        Tab => {
            let input = ed
                .minibuffer()
                .map(|mb| mb.input.clone())
                .unwrap_or_default();
            let completer = ed.minibuffer().and_then(|mb| mb.completion);
            let candidates = completer.map(|f| f(ed, &input)).unwrap_or_default();
            if let Some(mb) = ed.minibuffer_mut() {
                mb.complete_with(candidates);
            }
        }
        Backspace => {
            if let Some(mb) = ed.minibuffer_mut() {
                mb.delete_backward();
            }
        }
        Delete => {
            if let Some(mb) = ed.minibuffer_mut() {
                mb.delete_forward();
            }
        }
        Left => {
            if let Some(mb) = ed.minibuffer_mut() {
                mb.move_left();
            }
        }
        Right => {
            if let Some(mb) = ed.minibuffer_mut() {
                mb.move_right();
            }
        }
        Home => {
            if let Some(mb) = ed.minibuffer_mut() {
                mb.to_start();
            }
        }
        End => {
            if let Some(mb) = ed.minibuffer_mut() {
                mb.to_end();
            }
        }
        _ => {}
    }
    Ok(())
}

/// Keys while a continuation is pending (yes/no prompt, describe-key).
fn pending_key(ed: &mut Editor, key: Key) -> Result<()> {
    // describe-key: accumulate until the sequence resolves.
    if matches!(ed.pending(), Some(Pending::ReadKey { .. })) {
        let mut resolved = false;
        if let Some(Pending::ReadKey { keys }) = ed.pending_mut() {
            keys.push(key);
            let seq = keys.clone();
            match ed.keymap().lookup(&seq) {
                Lookup::Command(name) => {
                    let name = name.to_string();
                    let seqs: Vec<String> = seq.iter().map(|k| k.to_string()).collect();
                    let doc = ed.commands().get(&name).map(|c| c.doc).unwrap_or("");
                    let doc = if doc.is_empty() {
                        String::new()
                    } else {
                        format!(": {doc}")
                    };
                    ed.set_pending(None);
                    ed.message(format!(
                        "{} runs the command {}{}",
                        seqs.join(" "),
                        name,
                        doc
                    ));
                    resolved = true;
                }
                Lookup::Prefix => {
                    let seqs: Vec<String> = seq.iter().map(|k| k.to_string()).collect();
                    ed.message(format!("{}-", seqs.join(" ")));
                    resolved = true;
                }
                Lookup::Unbound => {
                    let seqs: Vec<String> = seq.iter().map(|k| k.to_string()).collect();
                    ed.set_pending(None);
                    ed.error(format!("{} is undefined", seqs.join(" ")));
                    resolved = true;
                }
            }
        }
        if resolved {
            return Ok(());
        }
        return Ok(());
    }

    // yes/no prompt
    let is_yesno = matches!(ed.pending(), Some(Pending::YesNo { .. }));
    if is_yesno {
        let answer = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
            KeyCode::Char('n') | KeyCode::Char('N') => Some(false),
            KeyCode::Char('g') if key.mods.contains(Modifiers::CONTROL) => None,
            _ => return Ok(()),
        };
        if answer.is_none() {
            ed.abort_pending();
            ed.message("Quit");
            return Ok(());
        }
        if let Some(Pending::YesNo { cont, .. }) = ed.take_pending() {
            let r = cont(ed, answer.unwrap());
            if let Err(e) = r {
                ed.error(e.to_string());
            }
        }
    }
    Ok(())
}
