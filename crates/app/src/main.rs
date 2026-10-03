//! emacs-rs: an Emacs-like editor with a rope-backed buffer.
//!
//! The Rust binary is a minimal core: terminal event loop, key dispatch,
//! rendering, and rope primitives. Commands, modes, undo, the kill ring,
//! isearch, and dired are implemented in Lua (the `lua/` runtime directory
//! plus the user's init.lua), driven as coroutines from this loop.

use std::io::{self, Stdout};
use std::path::PathBuf;

use anyhow::Result;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use emacs_core::editor::Editor;
use emacs_core::input::handle_key;
use emacs_core::key::{Key, KeyCode, Modifiers};
use emacs_core::watch::FileWatcher;
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

/// Restores the terminal if startup or the event loop exits unexpectedly.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
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
    let _terminal_guard = TerminalGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let size = terminal.size()?;

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

/// C-z (`suspend-frame`): hand the terminal back to the shell and stop
/// the process (job control resumes it via `fg`). The terminal is
/// restored before stopping and re-taken with a full redraw after.
fn suspend_frame(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, Show)?;
    // Stop only this editor process. The shell's `fg` sends SIGCONT when it
    // resumes the foreground job. Check both PID lookup and signal delivery;
    // silently continuing would leave the terminal in shell mode.
    let pid = unsafe { libc::getpid() };
    if pid <= 0 {
        return Err(anyhow::anyhow!("cannot determine editor PID"));
    }
    let rc = unsafe { libc::kill(pid, libc::SIGTSTP) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // Resumed: take the terminal back and redraw everything.
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, Hide)?;
    terminal.clear()?;
    Ok(())
}

fn run(ed: &mut Editor, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    // Watch buffer files with inotify (global-auto-revert-mode): events
    // arrive on `fw`'s channel and are drained each loop iteration.
    let mut fw = FileWatcher::new();
    fw.sync(ed);
    // How long the loop blocks for input: also bounds the latency of
    // inotify event delivery. Small enough to feel instant, large enough
    // to idle cheaply.
    const DRAIN: std::time::Duration = std::time::Duration::from_millis(250);
    loop {
        fw.sync(ed);
        ed.scroll_current_view();
        terminal.draw(|f| {
            if let Some((x, y)) = render(f, ed) {
                f.set_cursor_position(ratatui::layout::Position::new(x, y));
            }
        })?;
        if ed.quit() {
            return Ok(());
        }
        if ed.suspend_requested() {
            ed.clear_suspend_request();
            suspend_frame(terminal)?;
        }
        if event::poll(DRAIN)? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if let Some(key) = to_key(&k) {
                        if let Err(e) = handle_key(ed, key) {
                            ed.error(e.to_string());
                        }
                        fw.sync(ed);
                    }
                }
                Event::Resize(w, h) => {
                    ed.set_window_size(h.saturating_sub(2) as usize, w as usize);
                }
                _ => {}
            }
        }
        let changed = fw.drain();
        if !changed.is_empty() {
            if let Err(e) = ed.notify_file_changes(&changed) {
                ed.error(e.to_string());
            }
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
