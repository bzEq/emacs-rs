//! emacs-rs: an Emacs-like editor with a rope-backed buffer.
//!
//! The Rust binary is a minimal core: terminal event loop, key dispatch,
//! rendering, and rope primitives. Commands, modes, undo, the kill ring,
//! isearch, and dired are implemented in Lua (the `lua/` runtime directory
//! plus the user's init.lua), driven as coroutines from this loop.

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
use emacs_core::script::{CommandOutcome, PendingRequest, ResumeValue};
use emacs_lua::LuaHost;
use emacs_ui::render;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

/// Parsed command-line options.
struct CliArgs {
    /// Path of the init file (`--init`); falls back to the XDG config.
    init: Option<PathBuf>,
    /// First positional argument: file to open.
    file: Option<String>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<CliArgs, String> {
    let mut cli = CliArgs {
        init: None,
        file: None,
    };
    let mut it = args;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--init" => match it.next() {
                Some(path) if !path.is_empty() => cli.init = Some(PathBuf::from(path)),
                _ => return Err("--init requires a file path".into()),
            },
            a if a.starts_with("--") => return Err(format!("unknown option: {a}")),
            _ => {
                if cli.file.is_none() {
                    cli.file = Some(arg);
                }
            }
        }
    }
    Ok(cli)
}

fn main() -> Result<()> {
    let cli = match parse_args(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(e) => {
            eprintln!("em: {e}\nusage: em [--init <init.lua>] [FILE]");
            std::process::exit(2);
        }
    };
    let file_arg = cli.file;

    // The Lua runtime is embedded in the binary; `EMACS_RS_LUA_DIR` loads
    // it from a directory instead (developing the runtime). An explicit
    // override is validated before touching the terminal, so a broken one
    // is a clean fatal error instead of a broken editor.
    let runtime_dir = std::env::var_os("EMACS_RS_LUA_DIR").map(PathBuf::from);
    if let Some(dir) = &runtime_dir {
        if !dir.join(emacs_lua::RUNTIME_MODULES[0]).is_file() {
            fatal(&format!(
                "EMACS_RS_LUA_DIR is set to {}, but the Lua runtime (api.lua) is not there",
                dir.display()
            ));
        }
        if let Some(missing) = emacs_lua::RUNTIME_MODULES
            .iter()
            .find(|m| !dir.join(m).is_file())
        {
            fatal(&format!(
                "required Lua runtime module is missing: {}",
                dir.join(missing).display()
            ));
        }
    }

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

    let mut ed = Editor::new(size.height.saturating_sub(2) as usize, size.width as usize);

    // LuaJIT scripting engine: the embedded runtime (all default
    // commands, keybindings, and modes), then the user's init.lua.
    let host = match LuaHost::new() {
        Ok(host) => host,
        Err(e) => fatal(&format!("LuaJIT unavailable: {e}")),
    };
    ed.attach_script(Box::new(host));
    let runtime_result = match &runtime_dir {
        Some(dir) => ed.load_runtime_dir(dir),
        None => ed.load_runtime(),
    };
    if let Err(e) = runtime_result {
        fatal(&format!("failed to load the Lua runtime: {e}"));
    }
    let init = cli.init.clone().or_else(init_file);
    if let Some(init) = init {
        if init.exists() {
            if let Err(e) = ed.load_script(&init) {
                ed.error(format!("error loading {}: {e}", init.display()));
            }
        } else if cli.init.is_some() {
            ed.error(format!("cannot open init file: {}", init.display()));
        }
    }
    if let Err(e) = ed.run_startup(file_arg.as_deref()) {
        ed.error(e.to_string());
    }

    let result = run(&mut ed, &mut terminal);

    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    result
}

/// Report a fatal runtime error and exit: restore the terminal first, so
/// this is safe both before and after entering the alternate screen.
fn fatal(msg: &str) -> ! {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    eprintln!("em: fatal: {msg}");
    std::process::exit(1);
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
                    ed.refresh_syntax_current();
                }
            }
            Event::Resize(w, h) => {
                ed.set_window_size(h.saturating_sub(2) as usize, w as usize);
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
        // crossterm reports plain uppercase letters with a SHIFT modifier;
        // terminals cannot distinguish the two, and keymaps bind the plain
        // letter (dired's R/D/U/C, self-insert), so normalize it away.
        crossterm::event::KeyCode::Char(c)
            if c.is_ascii_uppercase() && mods == Modifiers::SHIFT =>
        {
            mods = Modifiers::empty();
            KeyCode::Char(c)
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

/// One key press into the current input state (a pending command request,
/// or the normal keymap dispatch).
fn handle_key(ed: &mut Editor, key: Key) -> Result<()> {
    if std::env::var_os("EM_DEBUG_KEYS").is_some() {
        eprintln!(
            "em: key code={:?} mods={:?} -> {key} (pending={:?})",
            key.code,
            key.mods,
            ed.pending()
        );
    }
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
    if ed.pending().is_some() {
        return pending_key(ed, key);
    }

    ed.clear_echo();
    ed.push_key(key);
    let seq = ed.pending_keys().to_vec();
    let name = match ed.lookup_key(&seq) {
        Lookup::Command(name) => Some(name),
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
    let extra = match (name.as_str(), key.code) {
        ("self-insert-command", KeyCode::Char(c)) => Some(c),
        _ => None,
    };
    run_command(ed, &name, extra);
    Ok(())
}

/// Run a Lua command; store any pending input request it makes.
fn run_command(ed: &mut Editor, name: &str, extra: Option<char>) {
    match ed.call_command(name, extra) {
        Ok(outcome) => {
            if let Some(key) = ed.finish_command(outcome) {
                let _ = dispatch(ed, key);
            }
        }
        Err(e) => {
            ed.finish_command(CommandOutcome::Done);
            ed.error(e.to_string());
        }
    }
}

/// Resume a suspended command coroutine with a value.
fn resume(ed: &mut Editor, value: ResumeValue) -> Result<()> {
    match ed.resume_pending(value) {
        Ok(outcome) => {
            if let Some(key) = ed.finish_command(outcome) {
                return dispatch(ed, key);
            }
            Ok(())
        }
        Err(e) => {
            ed.finish_command(CommandOutcome::Done);
            ed.error(e.to_string());
            Ok(())
        }
    }
}

/// Recompute completion candidates from the minibuffer input. When `fill`
/// is set the longest common prefix is auto-filled; after deletions it must
/// be false so the auto-fill doesn't re-insert deleted characters.
fn update_completion(ed: &mut Editor, fill: bool) {
    let has_completion = ed.minibuffer().is_some_and(|mb| mb.completion);
    if !has_completion {
        return;
    }
    let input = ed
        .minibuffer()
        .map(|mb| mb.input.clone())
        .unwrap_or_default();
    let candidates = ed.update_completion(&input).unwrap_or_default();
    if let Some(mb) = ed.minibuffer_mut() {
        mb.complete_with(candidates, fill);
    }
    // recompute against the (possibly extended) input so the displayed
    // candidates always match what is in the minibuffer
    let input = ed
        .minibuffer()
        .map(|mb| mb.input.clone())
        .unwrap_or_default();
    let candidates = ed.update_completion(&input).unwrap_or_default();
    if let Some(mb) = ed.minibuffer_mut() {
        mb.candidates = candidates;
    }
}

/// Keys while a command is waiting for input.
fn pending_key(ed: &mut Editor, key: Key) -> Result<()> {
    let request = ed.pending().cloned();
    match request {
        Some(PendingRequest::ReadString { .. }) => minibuffer_key(ed, key),
        Some(PendingRequest::ReadYesNo { .. }) => {
            let answer = match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
                KeyCode::Char('n') | KeyCode::Char('N') => Some(false),
                KeyCode::Char('g') if key.mods.contains(Modifiers::CONTROL) => None,
                _ => return Ok(()),
            };
            if answer.is_none() {
                ed.message("Quit");
                return resume(ed, ResumeValue::Bool(None));
            }
            resume(ed, ResumeValue::Bool(answer))
        }
        Some(PendingRequest::ReadKey) => {
            ed.set_read_key(key);
            resume(ed, ResumeValue::Key(key))
        }
        None => Ok(()),
    }
}

/// Keys while the minibuffer is reading input. The minibuffer-local
/// keymap overrides the global map (RET/TAB/C-g stay structural), and
/// everything else falls through to the global keymap, so user bindings
/// keep working while the minibuffer is active (Emacs behavior).
fn minibuffer_key(ed: &mut Editor, key: Key) -> Result<()> {
    use KeyCode::*;
    let m = key.mods;

    // structural keys: abort, accept, complete
    if matches!(key.code, Char('g') if m.contains(Modifiers::CONTROL)) {
        ed.clear_pending_keys();
        ed.message("Quit");
        return resume(ed, ResumeValue::String(None));
    }
    match key.code {
        Enter => {
            let input = ed.minibuffer().map(|mb| mb.accepted()).unwrap_or_default();
            ed.clear_pending_keys();
            ed.push_minibuffer_history(input.clone());
            return resume(ed, ResumeValue::String(Some(input)));
        }
        Tab => {
            let had_preview = ed.minibuffer().is_some_and(|mb| !mb.preview.is_empty());
            let prev_cands = ed
                .minibuffer()
                .map(|mb| mb.candidates.clone())
                .unwrap_or_default();
            if let Some(mb) = ed.minibuffer_mut() {
                mb.accept_preview();
            }
            update_completion(ed, true);
            if !had_preview {
                // nothing to accept: cycle through candidates instead
                if let Some(mb) = ed.minibuffer_mut() {
                    // after cycling to a full name the candidate set
                    // collapses to that one entry; keep cycling over the
                    // previous set instead
                    if mb.candidates.len() < 2 && prev_cands.len() >= 2 {
                        mb.candidates = prev_cands;
                        let pos = mb.candidates.iter().position(|c| *c == mb.input);
                        mb.cycle = match pos {
                            Some(i) => i, // cycle() advances to the next
                            None => usize::MAX,
                        };
                    }
                    mb.cycle();
                }
            }
            return Ok(());
        }
        _ => {}
    }

    // minibuffer-local keymap, then the global keymap, then self-insert
    ed.push_key(key);
    let seq = ed.pending_keys().to_vec();
    match ed.lookup_minibuffer_key(&seq) {
        Lookup::Command(name) => {
            ed.clear_pending_keys();
            // The command runs against the live minibuffer input state;
            // capture it afterwards (finish_command clears it) so the
            // read continues with the edits applied.
            let outcome = match ed.call_command(&name, None) {
                Ok(o) => o,
                Err(e) => {
                    ed.error(e.to_string());
                    CommandOutcome::Done
                }
            };
            let after = ed.minibuffer().cloned();
            if let Some(key) = ed.finish_command(outcome) {
                let _ = key; // commands run from the minibuffer don't replay
            }
            if ed.pending().is_none() {
                if let Some(mb) = after {
                    ed.restore_minibuffer(mb);
                }
            }
            update_completion(ed, false);
        }
        Lookup::Prefix => {}
        Lookup::Unbound => {
            let insert = ed.pending_keys().len() == 1 && key.is_self_insertable();
            ed.clear_pending_keys();
            if insert {
                if let Char(c) = key.code {
                    if let Some(mb) = ed.minibuffer_mut() {
                        mb.insert_char(c);
                    }
                    update_completion(ed, true);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(v: &[&str]) -> CliArgs {
        parse_args(v.iter().map(|s| s.to_string())).unwrap()
    }

    #[test]
    fn init_option() {
        let cli = parse(&["--init", "/tmp/foo.lua", "file.rs"]);
        assert_eq!(
            cli.init.as_deref(),
            Some(std::path::Path::new("/tmp/foo.lua"))
        );
        assert_eq!(cli.file.as_deref(), Some("file.rs"));
    }

    #[test]
    fn file_only() {
        let cli = parse(&["notes.txt"]);
        assert_eq!(cli.init, None);
        assert_eq!(cli.file.as_deref(), Some("notes.txt"));
    }

    #[test]
    fn no_args() {
        let cli = parse(&[]);
        assert_eq!(cli.init, None);
        assert_eq!(cli.file, None);
    }

    #[test]
    fn init_missing_path_errors() {
        assert!(parse_args(["--init".to_string()].into_iter()).is_err());
        assert!(parse_args(["--nope".to_string()].into_iter()).is_err());
    }
}
