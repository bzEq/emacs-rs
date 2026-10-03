//! The keyboard/command loop: one key press into the current input state
//! (a pending command request, or the normal keymap dispatch). This is the
//! core's equivalent of Emacs's `keyboard.c`: it knows how to route keys,
//! not what the commands do.
//!
//! The terminal adapter (`to_key`, the event loop) lives in `emacs-app`;
//! by the time a key gets here it is a terminal-independent [`Key`].

use anyhow::Result;

use crate::editor::Editor;
use crate::key::{Key, KeyCode, Modifiers};
use crate::keymap::Lookup;
use crate::script::{CommandOutcome, PendingRequest, ResumeValue};

/// One key press into the current input state (a pending command request,
/// or the normal keymap dispatch).
pub fn handle_key(ed: &mut Editor, key: Key) -> Result<()> {
    if std::env::var_os("EM_DEBUG_KEYS").is_some() {
        eprintln!(
            "em: key code={:?} mods={:?} -> {key} (pending={:?}, keys={:?}, esc={})",
            key.code,
            key.mods,
            ed.pending(),
            ed.pending_keys(),
            ed.esc_prefix()
        );
    }
    // A pending read-key owns Esc as data; only normal dispatch treats it as
    // the Meta prefix. Check before terminal Ctrl-X translations as well.
    if ed
        .pending()
        .is_some_and(|p| matches!(p, PendingRequest::ReadKey))
    {
        return dispatch(ed, key);
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
    let input = ed.minibuffer_input();
    let candidates = ed.update_completion(&input).unwrap_or_default();
    ed.minibuffer_complete_with(candidates, fill);
    // recompute against the (possibly extended) input so the displayed
    // candidates always match what is in the minibuffer
    let input = ed.minibuffer_input();
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
                ed.deactivate_mark();
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
        ed.deactivate_mark();
        ed.message("Quit");
        return resume(ed, ResumeValue::String(None));
    }
    match key.code {
        Enter => {
            let input = ed.minibuffer_accepted();
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
            ed.minibuffer_accept_preview();
            update_completion(ed, true);
            if !had_preview {
                // nothing to accept: cycle through candidates instead
                let input = ed.minibuffer_input();
                if let Some(mb) = ed.minibuffer_mut() {
                    // after cycling to a full name the candidate set
                    // collapses to that one entry; keep cycling over the
                    // previous set instead
                    if mb.candidates.len() < 2 && prev_cands.len() >= 2 {
                        mb.candidates = prev_cands;
                        let pos = mb.candidates.iter().position(|c| *c == input);
                        mb.cycle = match pos {
                            Some(i) => i, // cycle() advances to the next
                            None => usize::MAX,
                        };
                    }
                }
                ed.minibuffer_cycle();
            }
            return Ok(());
        }
        _ => {}
    }

    // the minibuffer's buffer-local keymap (minibuffer-mode), then the
    // global keymap, then self-insert
    ed.push_key(key);
    let seq = ed.pending_keys().to_vec();
    match ed.lookup_key(&seq) {
        Lookup::Command(name) => {
            ed.clear_pending_keys();
            // The command runs against the live minibuffer input state; the
            // read itself is untouched by finish_command, so the edits stay
            // and the read continues.  A requested replay is dispatched in
            // the minibuffer context.
            let outcome = match ed.call_command(&name, None) {
                Ok(o) => o,
                Err(e) => {
                    ed.error(e.to_string());
                    CommandOutcome::Done
                }
            };
            if let Some(key) = ed.finish_command(outcome) {
                return dispatch(ed, key);
            }
            update_completion(ed, false);
        }
        Lookup::Prefix => {}
        Lookup::Unbound => {
            let insert = ed.pending_keys().len() == 1 && key.is_self_insertable();
            ed.clear_pending_keys();
            if insert {
                if let Char(c) = key.code {
                    ed.minibuffer_insert_char(c);
                    update_completion(ed, true);
                }
            }
        }
    }
    Ok(())
}
