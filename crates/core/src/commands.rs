//! Built-in commands and the default global keymap.

use std::path::PathBuf;

use anyhow::{anyhow, Result};

use crate::buffer::Direction;
use crate::editor::{Editor, PrefixArg};
use crate::minibuffer::{complete_buffer_names, complete_command_names};

// --- helpers ---------------------------------------------------------------

fn n_repeat(ed: &Editor, default: usize) -> usize {
    let v = ed.prefix_arg().value();
    v.max(default as i64) as usize
}

/// Confirm save of a modified buffer; runs `cont` after confirmation.
#[allow(clippy::type_complexity)]
fn confirm_save(ed: &mut Editor, idx: usize, cont: Box<dyn FnOnce(&mut Editor) -> Result<()>>) {
    let name = ed.buffers()[idx].name().to_string();
    if ed.buffers()[idx].modified() {
        ed.read_yes_no(
            format!("Buffer {name} modified; save it? (y/n)"),
            Box::new(move |ed, yes| {
                if yes {
                    let res = ed.save_buffer_at(idx);
                    if res.is_ok() {
                        cont(ed)
                    } else {
                        res
                    }
                } else {
                    cont(ed)
                }
            }),
        );
    } else if let Err(e) = cont(ed) {
        ed.error(e.to_string());
    }
}

fn kill_current(ed: &mut Editor, text: String) {
    ed.kill(text);
}

// --- motion ----------------------------------------------------------------

fn forward_char(ed: &mut Editor) -> Result<()> {
    for _ in 0..n_repeat(ed, 1) {
        ed.buf_mut().move_char(Direction::Forward);
    }
    Ok(())
}

fn backward_char(ed: &mut Editor) -> Result<()> {
    for _ in 0..n_repeat(ed, 1) {
        ed.buf_mut().move_char(Direction::Backward);
    }
    Ok(())
}

fn next_line(ed: &mut Editor) -> Result<()> {
    for _ in 0..n_repeat(ed, 1) {
        ed.buf_mut().move_line(Direction::Forward);
    }
    Ok(())
}

fn previous_line(ed: &mut Editor) -> Result<()> {
    for _ in 0..n_repeat(ed, 1) {
        ed.buf_mut().move_line(Direction::Backward);
    }
    Ok(())
}

fn move_beginning_of_line(ed: &mut Editor) -> Result<()> {
    ed.buf_mut().move_to_line_start();
    Ok(())
}

fn move_end_of_line(ed: &mut Editor) -> Result<()> {
    ed.buf_mut().move_to_line_end();
    Ok(())
}

fn forward_word(ed: &mut Editor) -> Result<()> {
    for _ in 0..n_repeat(ed, 1) {
        ed.buf_mut().move_word(Direction::Forward);
    }
    Ok(())
}

fn backward_word(ed: &mut Editor) -> Result<()> {
    for _ in 0..n_repeat(ed, 1) {
        ed.buf_mut().move_word(Direction::Backward);
    }
    Ok(())
}

fn beginning_of_buffer(ed: &mut Editor) -> Result<()> {
    ed.buf_mut().move_to_buffer_start();
    Ok(())
}

fn end_of_buffer(ed: &mut Editor) -> Result<()> {
    ed.buf_mut().move_to_buffer_end();
    Ok(())
}

fn scroll_up_command(ed: &mut Editor) -> Result<()> {
    for _ in 0..n_repeat(ed, 1) {
        ed.page_down_current();
    }
    Ok(())
}

fn scroll_down_command(ed: &mut Editor) -> Result<()> {
    for _ in 0..n_repeat(ed, 1) {
        ed.page_up_current();
    }
    Ok(())
}

fn recenter(ed: &mut Editor) -> Result<()> {
    ed.recenter_current();
    Ok(())
}

// --- editing ---------------------------------------------------------------

fn self_insert_command(ed: &mut Editor) -> Result<()> {
    let c = ed
        .self_insert_char()
        .ok_or_else(|| anyhow!("self-insert-command without a character"))?;
    if ed.buf().read_only() {
        ed.error("Buffer is read-only");
        return Ok(());
    }
    for _ in 0..n_repeat(ed, 1) {
        ed.buf_mut().insert_char(c);
    }
    Ok(())
}

fn newline(ed: &mut Editor) -> Result<()> {
    ed.buf_mut().insert("\n");
    Ok(())
}

/// RET in programming modes: newline + auto-indentation.
fn newline_and_indent(ed: &mut Editor) -> Result<()> {
    crate::indent::newline_and_indent(ed);
    Ok(())
}

/// C-j: newline, indented unless point is inside a comment or string.
fn electric_newline_and_maybe_indent(ed: &mut Editor) -> Result<()> {
    crate::indent::electric_newline_and_maybe_indent(ed);
    Ok(())
}

/// TAB: re-indent the current line in programming modes, insert a tab
/// otherwise.
fn indent_for_tab_command(ed: &mut Editor) -> Result<()> {
    if ed.buf().mode().indent_unit.is_some() {
        crate::indent::indent_line(ed);
    } else {
        ed.buf_mut().insert_char('\t');
    }
    Ok(())
}

fn delete_char(ed: &mut Editor) -> Result<()> {
    for _ in 0..n_repeat(ed, 1) {
        ed.buf_mut().delete_forward();
    }
    Ok(())
}

fn backward_delete_char(ed: &mut Editor) -> Result<()> {
    for _ in 0..n_repeat(ed, 1) {
        if !crate::indent::backward_delete_indent(ed) {
            ed.buf_mut().delete_backward();
        }
    }
    Ok(())
}

/// Kill from point to end of line, or the line ending itself if point is
/// already at end of line. With a prefix arg, kills that many lines.
fn kill_line(ed: &mut Editor) -> Result<()> {
    let n = n_repeat(ed, 1);
    let mut killed = String::new();
    for _ in 0..n {
        let buf = ed.buf_mut();
        let line = buf.line_of_point();
        let line_start = buf.rope().line_to_char(line);
        let eol = line_start + buf.line_len_chars(line);
        if buf.point() == eol {
            let end = buf.step_right(eol);
            killed.push_str(&buf.delete_range(eol, end));
        } else {
            killed.push_str(&buf.delete_range(buf.point(), eol));
        }
    }
    kill_current(ed, killed);
    Ok(())
}

/// Kill from point to the end of the current word.
fn kill_word(ed: &mut Editor) -> Result<()> {
    let n = n_repeat(ed, 1);
    let mut killed = String::new();
    for _ in 0..n {
        let buf = ed.buf_mut();
        let start = buf.point();
        buf.move_word(Direction::Forward);
        let end = buf.point();
        killed.push_str(&buf.delete_range(start, end));
    }
    kill_current(ed, killed);
    Ok(())
}

fn backward_kill_word(ed: &mut Editor) -> Result<()> {
    let n = n_repeat(ed, 1);
    let mut killed = String::new();
    for _ in 0..n {
        let buf = ed.buf_mut();
        let end = buf.point();
        buf.move_word(Direction::Backward);
        let start = buf.point();
        killed.push_str(&buf.delete_range(start, end));
    }
    kill_current(ed, killed);
    Ok(())
}

fn kill_region(ed: &mut Editor) -> Result<()> {
    let Some((start, end)) = ed.buf().region() else {
        ed.error("The mark is not set now, so there is no region");
        return Ok(());
    };
    let text = ed.buf_mut().delete_range(start, end);
    kill_current(ed, text);
    Ok(())
}

fn kill_ring_save(ed: &mut Editor) -> Result<()> {
    let Some((start, end)) = ed.buf().region() else {
        ed.error("The mark is not set now, so there is no region");
        return Ok(());
    };
    let text = ed.buf().rope().slice(start..end).to_string();
    kill_current(ed, text);
    Ok(())
}

fn yank(ed: &mut Editor) -> Result<()> {
    let text = ed
        .kill_ring()
        .current()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("Kill ring is empty"))?;
    let pos = ed.buf().point();
    for _ in 0..n_repeat(ed, 1) {
        ed.buf_mut().insert(&text);
    }
    ed.set_last_yank(Some((pos, ed.buf().point() - pos)));
    Ok(())
}

fn yank_pop(ed: &mut Editor) -> Result<()> {
    if ed.last_command() != "yank" && ed.last_command() != "yank-pop" {
        return Err(anyhow!("Previous command was not a yank"));
    }
    if ed.kill_ring().len() < 2 {
        return Err(anyhow!("Only one element in the kill ring"));
    }
    let Some((pos, len)) = ed.last_yank() else {
        return Err(anyhow!("Previous command was not a yank"));
    };
    let text = ed
        .kill_ring_mut()
        .pop()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("Kill ring is empty"))?;
    let _ = ed.buf_mut().delete_range(pos, pos + len);
    ed.buf_mut().set_point(pos);
    ed.buf_mut().insert(&text);
    ed.set_last_yank(Some((pos, ed.buf().point() - pos)));
    Ok(())
}

fn undo(ed: &mut Editor) -> Result<()> {
    if !ed.buf_mut().undo() {
        ed.message("No further undo information");
    }
    Ok(())
}

fn set_mark_command(ed: &mut Editor) -> Result<()> {
    let p = ed.buf().point();
    ed.buf_mut().set_mark(Some(p));
    ed.message("Mark set");
    Ok(())
}

/// Move point to the beginning of line `n` (1-based), clamped to the last
/// line; sets the mark at the previous position (Emacs goto-line).
fn goto_line_number(ed: &mut Editor, n: usize) {
    let buf = ed.buf_mut();
    let old = buf.point();
    let line = n.saturating_sub(1).min(buf.len_lines() - 1);
    let start = buf.rope().line_to_char(line);
    buf.set_mark(Some(old));
    buf.set_point(start);
    let line = buf.line_of_point() + 1;
    ed.message(format!("Goto line {line}"));
}

fn goto_line(ed: &mut Editor) -> Result<()> {
    // a prefix argument supplies the line number directly
    if ed.prefix_arg().is_active() {
        let n = ed.prefix_arg().value().max(1) as usize;
        goto_line_number(ed, n);
        return Ok(());
    }
    let max = ed.buf().len_lines();
    ed.read_string(
        format!("Goto line (1-{max}): "),
        None,
        Box::new(|ed, input| {
            match input.trim().parse::<usize>() {
                Ok(n) => goto_line_number(ed, n),
                Err(_) => ed.error("not a number"),
            }
            Ok(())
        }),
    );
    Ok(())
}

fn exchange_point_and_mark(ed: &mut Editor) -> Result<()> {
    ed.buf_mut().exchange_point_and_mark();
    Ok(())
}

fn keyboard_quit(ed: &mut Editor) -> Result<()> {
    ed.abort_pending();
    ed.set_prefix_arg(PrefixArg::default());
    if ed.pending_keys().is_empty() && ed.minibuffer().is_none() {
        ed.message("Quit");
    }
    Ok(())
}

// --- windows ---------------------------------------------------------------

fn split_window_below(ed: &mut Editor) -> Result<()> {
    ed.split_window(crate::window::Split::Vertical);
    Ok(())
}

fn split_window_right(ed: &mut Editor) -> Result<()> {
    ed.split_window(crate::window::Split::Horizontal);
    Ok(())
}

fn delete_window(ed: &mut Editor) -> Result<()> {
    if !ed.delete_window() {
        ed.error("Attempt to delete the sole window");
    }
    Ok(())
}

fn delete_other_windows(ed: &mut Editor) -> Result<()> {
    ed.delete_other_windows();
    Ok(())
}

fn other_window(ed: &mut Editor) -> Result<()> {
    ed.other_window();
    Ok(())
}

// --- isearch ---------------------------------------------------------------

fn isearch_forward(ed: &mut Editor) -> Result<()> {
    ed.start_isearch(true);
    Ok(())
}

fn isearch_backward(ed: &mut Editor) -> Result<()> {
    ed.start_isearch(false);
    Ok(())
}

// --- major / minor modes ----------------------------------------------------

fn set_mode(ed: &mut Editor, name: &str) -> Result<()> {
    let idx = ed.selected_buffer_index();
    if let Err(e) = ed.set_buffer_mode_by_name(idx, name) {
        ed.error(e.to_string());
    }
    Ok(())
}

fn fundamental_mode(ed: &mut Editor) -> Result<()> {
    set_mode(ed, "fundamental-mode")
}

fn rust_mode(ed: &mut Editor) -> Result<()> {
    set_mode(ed, "rust-mode")
}

fn lua_mode(ed: &mut Editor) -> Result<()> {
    set_mode(ed, "lua-mode")
}

fn line_numbers_mode(ed: &mut Editor) -> Result<()> {
    let idx = ed.selected_buffer_index();
    match ed.toggle_minor_mode(idx, "line-numbers") {
        Ok(true) => ed.message("Line numbers enabled"),
        Ok(false) => ed.message("Line numbers disabled"),
        Err(e) => ed.error(e.to_string()),
    }
    Ok(())
}

fn universal_argument(ed: &mut Editor) -> Result<()> {
    let mut arg = ed.prefix_arg();
    arg.universal += 1;
    ed.set_prefix_arg(arg);
    Ok(())
}

fn digit_argument(ed: &mut Editor, digit: u64) {
    let mut arg = ed.prefix_arg();
    arg.digits = Some(arg.digits.unwrap_or(0) * 10 + digit);
    ed.set_prefix_arg(arg);
}

fn negative_argument(ed: &mut Editor) -> Result<()> {
    let mut arg = ed.prefix_arg();
    arg.negative = !arg.negative;
    ed.set_prefix_arg(arg);
    Ok(())
}

// --- files / buffers -------------------------------------------------------

fn find_file(ed: &mut Editor) -> Result<()> {
    let cur = ed.selected_buffer_index();
    ed.read_string(
        "Find file: ",
        None,
        Box::new(move |ed, name| {
            if name.is_empty() {
                return Ok(());
            }
            confirm_save(
                ed,
                cur,
                Box::new(move |ed| {
                    let path = PathBuf::from(name);
                    // opening a directory runs dired on it (Emacs behavior)
                    if path.is_dir() {
                        return crate::dired::open_dir(ed, &path, false);
                    }
                    if let Some(idx) = ed.find_buffer_by_path(&path) {
                        let id = ed.buffers()[idx].id;
                        ed.set_selected_buffer(id);
                        return Ok(());
                    }
                    match crate::buffer::Buffer::load_file(&path) {
                        Ok(buf) => {
                            let id = buf.id;
                            ed.add_buffer(buf);
                            ed.set_selected_buffer(id);
                            Ok(())
                        }
                        Err(e) => {
                            ed.error(format!("Cannot open {path:?}: {e}"));
                            Ok(())
                        }
                    }
                }),
            );
            Ok(())
        }),
    );
    Ok(())
}

fn save_buffer(ed: &mut Editor) -> Result<()> {
    let idx = ed.selected_buffer_index();
    ed.save_buffer_at(idx)
}

fn write_file(ed: &mut Editor) -> Result<()> {
    let cur_id = ed.selected_buffer_id();
    let default = ed.current_buf_path().unwrap_or_default();
    let prompt = format!("Write file: {} ", default.display());
    ed.read_string(
        prompt,
        None,
        Box::new(move |ed, name| {
            if name.is_empty() {
                return Ok(());
            }
            let path = PathBuf::from(&name);
            let file_name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.clone());
            let idx = ed.buffer_index(cur_id);
            ed.buffers_mut()[idx].set_path(Some(path));
            ed.buffers_mut()[idx].set_name(file_name);
            ed.save_buffer_at(idx)
        }),
    );
    Ok(())
}

fn switch_to_buffer(ed: &mut Editor) -> Result<()> {
    let cur = ed.selected_buffer_index();
    ed.read_string(
        "Switch to buffer: ",
        Some(complete_buffer_names),
        Box::new(move |ed, name| {
            if name.is_empty() {
                return Ok(());
            }
            confirm_save(
                ed,
                cur,
                Box::new(move |ed| {
                    if let Err(e) = ed.switch_to_buffer(&name, true) {
                        ed.error(e.to_string());
                    }
                    Ok(())
                }),
            );
            Ok(())
        }),
    );
    Ok(())
}

fn kill_buffer(ed: &mut Editor) -> Result<()> {
    let cur = ed.selected_buffer_index();
    let default = ed.buffers()[cur].name().to_string();
    let prompt = format!("Kill buffer (default {default}): ");
    ed.read_string(
        prompt,
        Some(complete_buffer_names),
        Box::new(move |ed, name| {
            let name = if name.is_empty() { default } else { name };
            let Some(idx) = ed.buffers().iter().position(|b| b.name() == name) else {
                ed.error(format!("no buffer named {name}"));
                return Ok(());
            };
            if ed.buffers()[idx].modified() {
                ed.read_yes_no(
                    format!("Buffer {name} modified; kill anyway? (y/n)"),
                    Box::new(move |ed, yes| {
                        if yes {
                            ed.kill_buffer_at(idx);
                        }
                        Ok(())
                    }),
                );
            } else {
                ed.kill_buffer_at(idx);
            }
            Ok(())
        }),
    );
    Ok(())
}

fn save_buffers_kill_terminal(ed: &mut Editor) -> Result<()> {
    confirm_all_modified(
        ed,
        0,
        Box::new(|ed| {
            ed.set_quit(true);
            Ok(())
        }),
    )
}

/// Ask about each modified buffer in turn, then run `cont`.
#[allow(clippy::type_complexity)]
fn confirm_all_modified(
    ed: &mut Editor,
    idx: usize,
    cont: Box<dyn FnOnce(&mut Editor) -> Result<()>>,
) -> Result<()> {
    let modified: Vec<usize> = ed
        .buffers()
        .iter()
        .enumerate()
        .filter(|(_, b)| b.modified())
        .map(|(i, _)| i)
        .collect();
    if idx >= modified.len() {
        return cont(ed);
    }
    let buf_idx = modified[idx];
    let name = ed.buffers()[buf_idx].name().to_string();
    ed.read_yes_no(
        format!("Buffer {name} modified; save it? (y/n)"),
        Box::new(move |ed, yes| {
            if yes {
                ed.save_buffer_at(buf_idx)?;
            }
            confirm_all_modified(ed, idx + 1, cont)
        }),
    );
    Ok(())
}

// --- misc ------------------------------------------------------------------

fn execute_extended_command(ed: &mut Editor) -> Result<()> {
    ed.read_string(
        "M-x ",
        Some(complete_command_names),
        Box::new(|ed, name| {
            if name.is_empty() {
                return Ok(());
            }
            if let Err(e) = ed.invoke_command(&name) {
                ed.error(e.to_string());
            }
            Ok(())
        }),
    );
    Ok(())
}

fn describe_key(ed: &mut Editor) -> Result<()> {
    ed.clear_pending_keys();
    ed.set_pending(Some(crate::minibuffer::Pending::ReadKey {
        keys: Vec::new(),
    }));
    Ok(())
}

fn describe_bindings(ed: &mut Editor) -> Result<()> {
    let mut text = String::from("Global key bindings:\n\n");
    for (seq, cmd) in ed.keymap().flatten() {
        let seqs: Vec<String> = seq.iter().map(|k| k.to_string()).collect();
        text.push_str(&format!("{}\t\t{}\n", seqs.join(" "), cmd));
    }
    let idx = ed.selected_buffer_index();
    if let Some(local) = ed.buffers()[idx].local_keymap() {
        text.push_str("\nLocal (mode) key bindings:\n\n");
        for (seq, cmd) in local.flatten() {
            let seqs: Vec<String> = seq.iter().map(|k| k.to_string()).collect();
            text.push_str(&format!("{}\t\t{}\n", seqs.join(" "), cmd));
        }
    }
    for name in ed.buffers()[idx].enabled_minor().to_vec() {
        if let Some(km) = ed.minor_def(&name).and_then(|d| d.keymap.as_ref()) {
            text.push_str(&format!("\nMinor mode {name} key bindings:\n\n"));
            for (seq, cmd) in km.flatten() {
                let seqs: Vec<String> = seq.iter().map(|k| k.to_string()).collect();
                text.push_str(&format!("{}\t\t{}\n", seqs.join(" "), cmd));
            }
        }
    }
    let id = if let Some(idx) = ed.buffers().iter().position(|b| b.name() == "*Help*") {
        ed.buffers()[idx].id
    } else {
        let mut buf = crate::buffer::Buffer::new("*Help*");
        buf.set_read_only(true);
        let id = buf.id;
        ed.add_buffer(buf);
        id
    };
    ed.set_selected_buffer(id);
    ed.buf_mut().set_modified(false);
    ed.buf_mut().move_to_buffer_start();
    ed.buf_mut().insert(&text);
    ed.buf_mut().move_to_buffer_start();
    ed.buf_mut().set_modified(false);
    Ok(())
}

// --- registry --------------------------------------------------------------

macro_rules! register {
    ($ed:expr, $name:literal, $fn:expr, $doc:literal) => {
        $ed.commands_mut().add($name, $doc, $fn)
    };
}

macro_rules! digit_cmd {
    ($ed:expr, $d:literal, $name:ident) => {
        fn $name(ed: &mut Editor) -> Result<()> {
            digit_argument(ed, $d);
            Ok(())
        }
        $ed.commands_mut().add(
            format!("digit-argument-{}", $d),
            "Start a numeric argument digit.",
            $name,
        );
    };
}

/// Register all built-in commands and the default global keymap.
pub fn register_defaults(ed: &mut Editor) {
    // motion
    register!(
        ed,
        "forward-char",
        forward_char,
        "Move point forward one character."
    );
    register!(
        ed,
        "backward-char",
        backward_char,
        "Move point backward one character."
    );
    register!(
        ed,
        "next-line",
        next_line,
        "Move point down one line, preserving column."
    );
    register!(
        ed,
        "previous-line",
        previous_line,
        "Move point up one line, preserving column."
    );
    register!(
        ed,
        "move-beginning-of-line",
        move_beginning_of_line,
        "Move point to beginning of line."
    );
    register!(
        ed,
        "move-end-of-line",
        move_end_of_line,
        "Move point to end of line."
    );
    register!(
        ed,
        "forward-word",
        forward_word,
        "Move point forward one word."
    );
    register!(
        ed,
        "backward-word",
        backward_word,
        "Move point backward one word."
    );
    register!(
        ed,
        "beginning-of-buffer",
        beginning_of_buffer,
        "Move point to beginning of buffer."
    );
    register!(
        ed,
        "end-of-buffer",
        end_of_buffer,
        "Move point to end of buffer."
    );
    register!(
        ed,
        "scroll-up-command",
        scroll_up_command,
        "Scroll text upward one screenful."
    );
    register!(
        ed,
        "scroll-down-command",
        scroll_down_command,
        "Scroll text downward one screenful."
    );
    register!(
        ed,
        "recenter-top-bottom",
        recenter,
        "Center point in the window."
    );
    // editing
    register!(
        ed,
        "self-insert-command",
        self_insert_command,
        "Insert the typed character."
    );
    register!(ed, "newline", newline, "Insert a newline.");
    register!(
        ed,
        "newline-and-indent",
        newline_and_indent,
        "Insert a newline and indent the new line."
    );
    register!(
        ed,
        "electric-newline-and-maybe-indent",
        electric_newline_and_maybe_indent,
        "Insert a newline and indent the new line, unless point is in a comment or string."
    );
    register!(
        ed,
        "indent-for-tab-command",
        indent_for_tab_command,
        "Indent the current line, or insert a tab."
    );
    register!(
        ed,
        "delete-char",
        delete_char,
        "Delete the character at point."
    );
    register!(
        ed,
        "backward-delete-char",
        backward_delete_char,
        "Delete the character before point."
    );
    register!(
        ed,
        "kill-line",
        kill_line,
        "Kill the rest of the current line."
    );
    register!(ed, "kill-word", kill_word, "Kill the word after point.");
    register!(
        ed,
        "backward-kill-word",
        backward_kill_word,
        "Kill the word before point."
    );
    register!(
        ed,
        "kill-region",
        kill_region,
        "Kill the text between point and mark."
    );
    register!(
        ed,
        "kill-ring-save",
        kill_ring_save,
        "Copy the region to the kill ring."
    );
    register!(ed, "yank", yank, "Insert the most recent kill.");
    register!(
        ed,
        "yank-pop",
        yank_pop,
        "Replace yanked text with a previous kill."
    );
    register!(ed, "undo", undo, "Undo the last change.");
    register!(
        ed,
        "set-mark-command",
        set_mark_command,
        "Set the mark where point is."
    );
    register!(
        ed,
        "goto-line",
        goto_line,
        "Move point to the beginning of a line, setting the mark."
    );
    register!(
        ed,
        "exchange-point-and-mark",
        exchange_point_and_mark,
        "Swap point and mark."
    );
    register!(
        ed,
        "keyboard-quit",
        keyboard_quit,
        "Abort the current operation."
    );
    register!(
        ed,
        "universal-argument",
        universal_argument,
        "Begin a numeric argument (4x)."
    );
    register!(
        ed,
        "negative-argument",
        negative_argument,
        "Begin a negative numeric argument."
    );
    register!(
        ed,
        "split-window-below",
        split_window_below,
        "Split the selected window in two, one above the other."
    );
    register!(
        ed,
        "split-window-right",
        split_window_right,
        "Split the selected window in two, side by side."
    );
    register!(
        ed,
        "delete-window",
        delete_window,
        "Delete the selected window."
    );
    register!(
        ed,
        "delete-other-windows",
        delete_other_windows,
        "Make the selected window fill its frame."
    );
    register!(ed, "other-window", other_window, "Select the next window.");
    register!(
        ed,
        "isearch-forward",
        isearch_forward,
        "Incremental search forward."
    );
    register!(
        ed,
        "isearch-backward",
        isearch_backward,
        "Incremental search backward."
    );
    register!(
        ed,
        "fundamental-mode",
        fundamental_mode,
        "Set the buffer to fundamental mode."
    );
    register!(ed, "rust-mode", rust_mode, "Set the buffer to rust mode.");
    register!(ed, "lua-mode", lua_mode, "Set the buffer to lua mode.");
    register!(
        ed,
        "line-numbers-mode",
        line_numbers_mode,
        "Toggle line numbers in the gutter."
    );
    // files
    register!(ed, "find-file", find_file, "Open a file.");
    register!(
        ed,
        "save-buffer",
        save_buffer,
        "Save the current buffer to its file."
    );
    register!(
        ed,
        "write-file",
        write_file,
        "Save the current buffer to a new file."
    );
    register!(
        ed,
        "switch-to-buffer",
        switch_to_buffer,
        "Switch to another buffer."
    );
    register!(ed, "kill-buffer", kill_buffer, "Kill a buffer.");
    register!(
        ed,
        "save-buffers-kill-terminal",
        save_buffers_kill_terminal,
        "Save buffers and exit."
    );
    // misc
    register!(
        ed,
        "execute-extended-command",
        execute_extended_command,
        "Run a command by name."
    );
    register!(
        ed,
        "describe-key",
        describe_key,
        "Show what a key sequence runs."
    );
    register!(
        ed,
        "describe-bindings",
        describe_bindings,
        "List all key bindings."
    );

    // digit arguments: C-1..C-9, M-1..M-9
    digit_cmd!(ed, 1, digit_1);
    digit_cmd!(ed, 2, digit_2);
    digit_cmd!(ed, 3, digit_3);
    digit_cmd!(ed, 4, digit_4);
    digit_cmd!(ed, 5, digit_5);
    digit_cmd!(ed, 6, digit_6);
    digit_cmd!(ed, 7, digit_7);
    digit_cmd!(ed, 8, digit_8);
    digit_cmd!(ed, 9, digit_9);

    let km = ed.keymap_mut();
    let b = |km: &mut crate::keymap::Keymap, seq: &str, cmd: &str| {
        km.bind_sequence(&crate::key::parse_sequence(seq).unwrap(), cmd);
    };
    // motion
    b(km, "C-f", "forward-char");
    b(km, "<right>", "forward-char");
    b(km, "C-b", "backward-char");
    b(km, "<left>", "backward-char");
    b(km, "C-n", "next-line");
    b(km, "<down>", "next-line");
    b(km, "C-p", "previous-line");
    b(km, "<up>", "previous-line");
    b(km, "C-a", "move-beginning-of-line");
    b(km, "<home>", "move-beginning-of-line");
    b(km, "C-e", "move-end-of-line");
    b(km, "<end>", "move-end-of-line");
    b(km, "M-f", "forward-word");
    b(km, "M-b", "backward-word");
    b(km, "M-<", "beginning-of-buffer");
    b(km, "M->", "end-of-buffer");
    b(km, "C-v", "scroll-up-command");
    b(km, "<next>", "scroll-up-command");
    b(km, "M-v", "scroll-down-command");
    b(km, "<prior>", "scroll-down-command");
    b(km, "C-l", "recenter-top-bottom");
    // editing
    b(km, "RET", "newline-and-indent");
    b(km, "C-j", "electric-newline-and-maybe-indent");
    b(km, "TAB", "indent-for-tab-command");
    b(km, "C-d", "delete-char");
    b(km, "<delete>", "delete-char");
    b(km, "DEL", "backward-delete-char");
    b(km, "C-k", "kill-line");
    b(km, "M-d", "kill-word");
    b(km, "M-DEL", "backward-kill-word");
    b(km, "C-w", "kill-region");
    b(km, "M-w", "kill-ring-save");
    b(km, "C-y", "yank");
    b(km, "M-y", "yank-pop");
    b(km, "C-/", "undo");
    b(km, "C-_", "undo");
    b(km, "C-SPC", "set-mark-command");
    b(km, "C-@", "set-mark-command");
    b(km, "C-g", "keyboard-quit");
    b(km, "M-g M-g", "goto-line");
    b(km, "C-u", "universal-argument");
    b(km, "C--", "negative-argument");
    b(km, "M--", "negative-argument");
    // files
    b(km, "C-x C-f", "find-file");
    b(km, "C-x C-s", "save-buffer");
    b(km, "C-x C-w", "write-file");
    b(km, "C-x b", "switch-to-buffer");
    b(km, "C-x k", "kill-buffer");
    b(km, "C-x u", "undo");
    b(km, "C-x C-x", "exchange-point-and-mark");
    b(km, "C-x C-c", "save-buffers-kill-terminal");
    // misc
    b(km, "M-x", "execute-extended-command");
    b(km, "C-h k", "describe-key");
    b(km, "C-h b", "describe-bindings");
    // windows
    b(km, "C-x 2", "split-window-below");
    b(km, "C-x 3", "split-window-right");
    b(km, "C-x 0", "delete-window");
    b(km, "C-x 1", "delete-other-windows");
    b(km, "C-x o", "other-window");
    // dired
    b(km, "C-x d", "dired");
    // search
    b(km, "C-s", "isearch-forward");
    b(km, "C-r", "isearch-backward");
    for d in 1..=9u64 {
        let c = (b'0' + d as u8) as char;
        b(km, &format!("C-{c}"), &format!("digit-argument-{d}"));
        b(km, &format!("M-{c}"), &format!("digit-argument-{d}"));
    }

    crate::dired::register(ed);
}

impl Editor {
    /// Save the buffer at `idx`, asking for a file name if it has none.
    pub fn save_buffer_at(&mut self, idx: usize) -> Result<()> {
        if self.buffers()[idx].path().is_none() {
            let id = self.buffers()[idx].id;
            self.read_string(
                "File to save in: ",
                None,
                Box::new(move |ed, name| {
                    if name.is_empty() {
                        return Ok(());
                    }
                    let path = PathBuf::from(&name);
                    let file_name = path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| name.clone());
                    let idx = ed.buffer_index(id);
                    ed.buffers_mut()[idx].set_path(Some(path));
                    ed.buffers_mut()[idx].set_name(file_name);
                    ed.save_buffer_now(idx)
                }),
            );
            Ok(())
        } else {
            self.save_buffer_now(idx)
        }
    }

    /// Save the buffer at `idx` to its file, running save hooks.
    pub fn save_buffer_now(&mut self, idx: usize) -> Result<()> {
        self.run_hook("before_save")?;
        let result = self.buffers()[idx].save();
        match result {
            Ok(()) => {
                self.buffers_mut()[idx].set_modified(false);
                self.run_hook("after_save")?;
                let path = self.buffers()[idx]
                    .path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                self.message(format!("Wrote {path}"));
                Ok(())
            }
            Err(e) => {
                self.error(format!("Cannot save: {e}"));
                Ok(())
            }
        }
    }

    /// Kill the buffer at `idx`, pointing any windows that displayed it at
    /// another buffer.
    pub fn kill_buffer_at(&mut self, idx: usize) {
        let kill_id = self.buffers()[idx].id;
        self.remove_buffer(idx);
        if self.buffers().is_empty() {
            let scratch = crate::buffer::Buffer::new("*scratch*");
            let sid = scratch.id;
            self.add_buffer(scratch);
            self.replace_buffer_in_windows(kill_id, sid);
        } else {
            let keep_id = self.buffers()[0].id;
            self.replace_buffer_in_windows(kill_id, keep_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_line_simple() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("hello\nworld");
        ed.buf_mut().move_to_buffer_start();
        ed.buf_mut().move_char(Direction::Forward);
        ed.invoke_command("kill-line").unwrap();
        assert_eq!(ed.buf().rope().to_string(), "h\nworld");
        assert_eq!(ed.kill_ring().current(), Some("ello"));
    }

    #[test]
    fn kill_line_at_eol_kills_newline() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("hello\nworld");
        ed.buf_mut().move_to_buffer_start();
        ed.buf_mut().move_to_line_end();
        ed.invoke_command("kill-line").unwrap();
        assert_eq!(ed.buf().rope().to_string(), "helloworld");
    }

    #[test]
    fn consecutive_kills_accumulate() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("abc def");
        ed.buf_mut().move_to_buffer_start();
        ed.invoke_command("kill-word").unwrap();
        ed.invoke_command("kill-word").unwrap();
        assert_eq!(ed.kill_ring().current(), Some("abc def"));
    }

    #[test]
    fn yank_and_pop() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("hello");
        ed.buf_mut().move_to_buffer_start();
        ed.invoke_command("set-mark-command").unwrap();
        ed.buf_mut().move_to_buffer_end();
        ed.invoke_command("kill-region").unwrap();
        ed.invoke_command("yank").unwrap();
        assert_eq!(ed.buf().rope().to_string(), "hello");
        // single kill-ring entry: yank-pop errors
        ed.invoke_command("yank-pop").unwrap_err();
    }

    #[test]
    fn undo_via_command() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("x");
        ed.invoke_command("undo").unwrap();
        assert!(ed.buf().rope().to_string().is_empty());
    }

    #[test]
    fn region_kill() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("hello world");
        ed.buf_mut().move_to_buffer_start();
        ed.invoke_command("set-mark-command").unwrap();
        ed.buf_mut().move_word(Direction::Forward);
        ed.invoke_command("kill-region").unwrap();
        assert_eq!(ed.buf().rope().to_string(), " world");
        assert_eq!(ed.kill_ring().current(), Some("hello"));
    }

    #[test]
    fn prefix_arg_motion() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("abcdef");
        ed.buf_mut().move_to_buffer_start();
        ed.invoke_command("digit-argument-3").unwrap();
        ed.invoke_command("forward-char").unwrap();
        assert_eq!(ed.buf().point(), 3);
    }

    #[test]
    fn goto_line_via_prefix_arg() {
        let mut ed = Editor::new(20, 80);
        let content: String = (1..=10).map(|i| format!("line {i}\n")).collect();
        ed.buf_mut().insert(&content);
        ed.buf_mut().move_to_buffer_start();
        ed.buf_mut().move_char(Direction::Forward);
        ed.invoke_command("digit-argument-5").unwrap();
        ed.invoke_command("goto-line").unwrap();
        assert_eq!(ed.buf().line_of_point(), 4, "line 5 (0-based 4)");
        assert_eq!(ed.buf().column(), 0, "at the beginning of the line");
        assert_eq!(ed.buf().mark(), Some(1), "mark at the previous position");
    }

    #[test]
    fn goto_line_clamps_to_last_line() {
        let mut ed = Editor::new(20, 80);
        ed.buf_mut().insert("a\nb\n");
        ed.invoke_command("digit-argument-9").unwrap();
        ed.invoke_command("digit-argument-9").unwrap(); // 99
        ed.invoke_command("goto-line").unwrap();
        assert_eq!(ed.buf().line_of_point(), ed.buf().len_lines() - 1);
    }
}
