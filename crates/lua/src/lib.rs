//! LuaJIT scripting host.
//!
//! Two roles:
//!
//! 1. **Primitive API**: installs a `raw` table of editor primitives (rope
//!    editing, motion, buffers, windows, keymaps, modes, filesystem) that
//!    the Lua runtime (`lua/` defaults modules) builds on.
//! 2. **Command driver**: runs Lua commands as coroutines. A command that
//!    needs input (minibuffer string, yes/no, a raw key) suspends via
//!    `coroutine.yield`; the Rust event loop resumes it once the input
//!    arrives. This gives Lua commands synchronous reads.
//!
//! # Safety of the editor reference
//!
//! Lua code runs only on the main thread, inside host methods that receive
//! `&mut Editor`. Each such call stores a raw pointer to the editor in Lua
//! app-data for the duration of the call and removes it afterwards, so the
//! `raw.*` API can reach the editor from Lua callbacks. During Lua
//! execution the surrounding `&mut Editor` is not accessed, so no aliasing
//! occurs. The editor must not move while a call is in flight (it is owned
//! by the app's main function, so this holds).

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use mlua::thread::ThreadStatus;
use mlua::{Function, Lua, Table, Thread, Value};

use emacs_core::editor::Editor;
use emacs_core::key::Key;
use emacs_core::keymap::{Keymap, Lookup};
use emacs_core::mode::Lang;
use emacs_core::script::{CommandOutcome, PendingRequest, ResumeValue, ScriptHost};

/// The Lua runtime modules, loaded in this exact order. Every module is
/// required: a missing or broken one is a fatal error, reported by the
/// app before the editor starts.
pub const RUNTIME_MODULES: &[&str] = &[
    "api.lua",
    "motion.lua",
    "editing.lua",
    "search.lua",
    "windows.lua",
    "files.lua",
    "modes.lua",
    "dired.lua",
    "help.lua",
    "bindings.lua",
];

/// Locate the Lua runtime directory: `EMACS_RS_LUA_DIR`, a `lua` directory
/// next to the executable, `../lua` relative to it (the repo layout when
/// running from `target/`), or the source tree `lua/` (dev builds).
///
/// A candidate is only accepted if it contains the runtime entry point
/// (`api.lua`), so a stale or unrelated `lua/` directory near the binary
/// cannot silently shadow the real runtime.
pub fn find_runtime() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("EMACS_RS_LUA_DIR") {
        let dir = PathBuf::from(dir);
        if runtime_entry(&dir) {
            return Ok(dir);
        }
        return Err(format!(
            "EMACS_RS_LUA_DIR is set to {}, but the Lua runtime (api.lua) is not there",
            dir.display()
        ));
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(base) = exe.parent() {
            candidates.push(base.join("lua"));
            if let Some(parent) = base.parent() {
                candidates.push(parent.join("lua"));
            }
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("lua"),
    );
    for dir in candidates {
        if runtime_entry(&dir) {
            return Ok(dir);
        }
    }
    Err(
        "cannot find the Lua runtime: searched lua/ next to the executable, ../lua, \
         and the source tree; set EMACS_RS_LUA_DIR"
            .into(),
    )
}

fn runtime_entry(dir: &Path) -> bool {
    dir.join(RUNTIME_MODULES[0]).is_file()
}

struct EditorRef(*mut Editor);

unsafe impl Send for EditorRef {}

/// The editor pointer is stored in Lua app-data for the duration of each
/// host call (see the safety notes at the top of this file). Lua runs on
/// the main thread and the surrounding `&mut Editor` is not accessed while
/// Lua code executes, so recovering `&mut Editor` from `&Lua` here is
/// sound; the lint does not know about the app-data discipline.
#[allow(clippy::mut_from_ref)]
fn editor_ref(lua: &Lua) -> mlua::Result<&mut Editor> {
    let guard = lua.app_data_ref::<Option<EditorRef>>();
    match guard {
        Some(opt) => match opt.as_ref() {
            Some(r) => Ok(unsafe { &mut *r.0 }),
            None => Err(mlua::Error::RuntimeError("editor unavailable".into())),
        },
        None => Err(mlua::Error::RuntimeError("editor unavailable".into())),
    }
}

fn anyhow_err(e: mlua::Error) -> anyhow::Error {
    anyhow!("{e}")
}

/// Run `f` with the editor reachable from Lua callbacks.
fn with_editor<T>(
    lua: &Lua,
    editor: &mut Editor,
    f: impl FnOnce(&Lua) -> mlua::Result<T>,
) -> Result<T> {
    lua.set_app_data::<Option<EditorRef>>(Some(EditorRef(editor)));
    let r = f(lua).map_err(anyhow_err);
    lua.set_app_data::<Option<EditorRef>>(None);
    r
}

/// Build a keymap from a Lua table of `{ ["key seq"] = "command", ... }`.
fn parse_keymap_table(_lua: &Lua, table: Option<Table>) -> mlua::Result<Option<Keymap>> {
    let Some(t) = table else {
        return Ok(None);
    };
    let mut km = Keymap::new();
    for pair in t.pairs::<String, String>() {
        let (seq, cmd) = pair?;
        let keys = emacs_core::key::parse_sequence(&seq).map_err(mlua::Error::RuntimeError)?;
        km.bind_sequence(&keys, &cmd);
    }
    if km.is_empty() {
        Ok(None)
    } else {
        Ok(Some(km))
    }
}

/// The `_internals` table installed by the Lua runtime; a clear error when
/// the runtime never loaded (bad or missing `lua/` directory).
fn internals_table(lua: &Lua) -> mlua::Result<Table> {
    lua.globals()
        .get::<Option<Table>>("_internals")?
        .ok_or_else(|| {
            mlua::Error::RuntimeError(
                "the Lua runtime did not load (_internals is missing); \
                 check the lua/ runtime directory"
                    .into(),
            )
        })
}

/// The `_internals.run_command(name, extra)` entry point installed by the
/// Lua runtime.
fn run_command_fn(lua: &Lua) -> mlua::Result<Function> {
    internals_table(lua)?.get("run_command")
}

/// The `_internals.startup(path)` entry point installed by the Lua runtime.
fn startup_fn(lua: &Lua) -> mlua::Result<Function> {
    internals_table(lua)?.get("startup")
}

/// Decode the result of resuming a command coroutine: either the command
/// finished, or it yielded a request for input.
fn interpret(thread: Thread, v: Value) -> mlua::Result<(CommandOutcome, Option<Function>)> {
    match thread.status() {
        ThreadStatus::Resumable => {
            let t: Table = match v {
                Value::Table(t) => t,
                _ => {
                    return Err(mlua::Error::RuntimeError(
                        "command yield must be a request table".into(),
                    ));
                }
            };
            let ty: String = t.get("type")?;
            let (outcome, completion) = match ty.as_str() {
                "read_string" => {
                    let prompt: String = t.get("prompt")?;
                    let completion: Option<Function> = t.get("completion")?;
                    (
                        CommandOutcome::Pending(PendingRequest::ReadString {
                            prompt,
                            completion: completion.is_some(),
                        }),
                        completion,
                    )
                }
                "read_yes_no" => {
                    let prompt: String = t.get("prompt")?;
                    (
                        CommandOutcome::Pending(PendingRequest::ReadYesNo { prompt }),
                        None,
                    )
                }
                "read_key" => (CommandOutcome::Pending(PendingRequest::ReadKey), None),
                other => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "unknown yield request: {other}"
                    )));
                }
            };
            Ok((outcome, completion))
        }
        _ => Ok((CommandOutcome::Done, None)),
    }
}

/// A suspended command coroutine and the completion callback of its current
/// minibuffer read, if any.
struct PendingThread {
    thread: Thread,
    completion: Option<Function>,
}

pub struct LuaHost {
    lua: Lua,
    pending: Option<PendingThread>,
}

impl LuaHost {
    pub fn new() -> Result<Self> {
        let lua = Lua::new();
        let mut host = LuaHost { lua, pending: None };
        host.install_api().map_err(anyhow_err)?;
        Ok(host)
    }

    /// Start a command in a fresh coroutine and run it until it finishes or
    /// yields.
    fn start_command(
        &mut self,
        name: &str,
        extra: Option<char>,
        editor: &mut Editor,
    ) -> Result<CommandOutcome> {
        let (outcome, completion, thread) = with_editor(&self.lua, editor, |lua| {
            let run = run_command_fn(lua)?;
            let thread = lua.create_thread(run)?;
            let v: Value = match extra {
                Some(c) => thread.resume((name, c.to_string()))?,
                None => thread.resume(name)?,
            };
            let (outcome, completion) = interpret(thread.clone(), v)?;
            Ok((outcome, completion, thread))
        })?;
        if matches!(outcome, CommandOutcome::Pending(_)) {
            self.pending = Some(PendingThread { thread, completion });
        }
        Ok(outcome)
    }

    /// Resume the suspended coroutine with the requested input.
    fn resume_coroutine(
        &mut self,
        value: ResumeValue,
        editor: &mut Editor,
    ) -> Result<CommandOutcome> {
        let pt = self
            .pending
            .take()
            .ok_or_else(|| anyhow!("no pending command to resume"))?;
        let (outcome, completion) = with_editor(&self.lua, editor, |_lua| {
            let v: Value = match value {
                ResumeValue::String(Some(s)) => pt.thread.resume(s)?,
                ResumeValue::String(None) => pt.thread.resume(())?,
                ResumeValue::Bool(b) => pt.thread.resume(b)?,
                ResumeValue::Key(k) => pt.thread.resume(k.to_string())?,
            };
            interpret(pt.thread.clone(), v)
        })?;
        if matches!(outcome, CommandOutcome::Pending(_)) {
            self.pending = Some(PendingThread {
                thread: pt.thread,
                completion,
            });
        }
        Ok(outcome)
    }

    // --- primitive API -----------------------------------------------------

    fn install_api(&mut self) -> mlua::Result<()> {
        let lua = &self.lua;
        let globals = lua.globals();
        let raw = lua.create_table()?;

        // -- current-buffer text editing -----------------------------------
        raw.set(
            "insert",
            lua.create_function(|lua, text: String| {
                editor_ref(lua)?.buf_mut().insert(&text);
                Ok(())
            })?,
        )?;
        raw.set(
            "insert_at",
            lua.create_function(|lua, (pos, text): (usize, String)| {
                let ed = editor_ref(lua)?;
                let len = text.chars().count();
                ed.buf_mut().insert_at(pos, &text);
                Ok(pos + len)
            })?,
        )?;
        raw.set(
            "delete_range",
            lua.create_function(|lua, (start, end): (usize, usize)| {
                Ok(editor_ref(lua)?.buf_mut().delete_range(start, end))
            })?,
        )?;
        raw.set(
            "delete_backward",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf_mut().delete_backward()))?,
        )?;
        raw.set(
            "delete_forward",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf_mut().delete_forward()))?,
        )?;
        raw.set(
            "point",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().point()))?,
        )?;
        raw.set(
            "set_point",
            lua.create_function(|lua, n: usize| {
                editor_ref(lua)?.buf_mut().set_point(n);
                Ok(())
            })?,
        )?;
        raw.set(
            "mark",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().mark()))?,
        )?;
        raw.set(
            "set_mark",
            lua.create_function(|lua, n: Option<usize>| {
                editor_ref(lua)?.buf_mut().set_mark(n);
                Ok(())
            })?,
        )?;
        raw.set(
            "region",
            lua.create_function(|lua, ()| {
                Ok(match editor_ref(lua)?.buf().region() {
                    Some((a, b)) => (Some(a), Some(b)),
                    None => (None, None),
                })
            })?,
        )?;
        raw.set(
            "move_char",
            lua.create_function(|lua, dir: String| {
                let d = match dir.as_str() {
                    "forward" => emacs_core::buffer::Direction::Forward,
                    "backward" => emacs_core::buffer::Direction::Backward,
                    _ => return Err(mlua::Error::RuntimeError("bad direction".into())),
                };
                editor_ref(lua)?.buf_mut().move_char(d);
                Ok(())
            })?,
        )?;
        raw.set(
            "move_word",
            lua.create_function(|lua, dir: String| {
                let d = match dir.as_str() {
                    "forward" => emacs_core::buffer::Direction::Forward,
                    "backward" => emacs_core::buffer::Direction::Backward,
                    _ => return Err(mlua::Error::RuntimeError("bad direction".into())),
                };
                editor_ref(lua)?.buf_mut().move_word(d);
                Ok(())
            })?,
        )?;
        raw.set(
            "move_line",
            lua.create_function(|lua, dir: String| {
                let d = match dir.as_str() {
                    "forward" => emacs_core::buffer::Direction::Forward,
                    "backward" => emacs_core::buffer::Direction::Backward,
                    _ => return Err(mlua::Error::RuntimeError("bad direction".into())),
                };
                editor_ref(lua)?.buf_mut().move_line(d);
                Ok(())
            })?,
        )?;
        raw.set(
            "move_line_start",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.buf_mut().move_to_line_start();
                Ok(())
            })?,
        )?;
        raw.set(
            "move_line_end",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.buf_mut().move_to_line_end();
                Ok(())
            })?,
        )?;
        raw.set(
            "move_buffer_start",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.buf_mut().move_to_buffer_start();
                Ok(())
            })?,
        )?;
        raw.set(
            "move_buffer_end",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.buf_mut().move_to_buffer_end();
                Ok(())
            })?,
        )?;
        raw.set(
            "move_to_line",
            lua.create_function(|lua, n: usize| {
                editor_ref(lua)?.buf_mut().move_to_line(n);
                Ok(())
            })?,
        )?;
        raw.set(
            "exchange_point_and_mark",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.buf_mut().exchange_point_and_mark();
                Ok(())
            })?,
        )?;
        raw.set(
            "line_of_point",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().line_of_point()))?,
        )?;
        raw.set(
            "column",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().column()))?,
        )?;
        raw.set(
            "len_chars",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().len_chars()))?,
        )?;
        raw.set(
            "len_lines",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().len_lines()))?,
        )?;
        raw.set(
            "line_text",
            lua.create_function(|lua, idx: usize| {
                let ed = editor_ref(lua)?;
                let idx = idx.min(ed.buf().len_lines() - 1);
                let start = ed.buf().rope().line_to_char(idx);
                let len = ed.buf().line_len_chars(idx);
                Ok(ed.buf().rope().slice(start..start + len).to_string())
            })?,
        )?;
        raw.set(
            "line_start",
            lua.create_function(|lua, idx: usize| {
                let ed = editor_ref(lua)?;
                let line = idx.min(ed.buf().len_lines() - 1);
                Ok(ed.buf().rope().line_to_char(line))
            })?,
        )?;
        raw.set(
            "line_len",
            lua.create_function(|lua, idx: usize| {
                let ed = editor_ref(lua)?;
                let line = idx.min(ed.buf().len_lines() - 1);
                Ok(ed.buf().line_len_chars(line))
            })?,
        )?;
        raw.set(
            "get_text",
            lua.create_function(|lua, (start, end): (usize, usize)| {
                let ed = editor_ref(lua)?;
                let rope = ed.buf().rope();
                let start = start.min(rope.len_chars());
                let end = end.min(rope.len_chars());
                Ok(rope.slice(start.min(end)..start.max(end)).to_string())
            })?,
        )?;
        raw.set(
            "buffer_string",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().rope().to_string()))?,
        )?;
        raw.set(
            "char_at",
            lua.create_function(|lua, idx: usize| {
                let ed = editor_ref(lua)?;
                let rope = ed.buf().rope();
                if idx >= rope.len_chars() {
                    return Ok(None);
                }
                Ok(Some(rope.char(idx).to_string()))
            })?,
        )?;
        raw.set(
            "step_right",
            lua.create_function(|lua, idx: usize| Ok(editor_ref(lua)?.buf().step_right(idx)))?,
        )?;
        raw.set(
            "step_left",
            lua.create_function(|lua, idx: usize| Ok(editor_ref(lua)?.buf().step_left(idx)))?,
        )?;
        raw.set(
            "search_forward",
            lua.create_function(|lua, (query, from): (String, usize)| {
                Ok(editor_ref(lua)?.search_forward(&query, from))
            })?,
        )?;
        raw.set(
            "search_backward",
            lua.create_function(|lua, (query, from): (String, usize)| {
                Ok(editor_ref(lua)?.search_backward(&query, from))
            })?,
        )?;
        raw.set(
            "read_only",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().read_only()))?,
        )?;
        raw.set(
            "set_read_only",
            lua.create_function(|lua, b: bool| {
                editor_ref(lua)?.buf_mut().set_read_only(b);
                Ok(())
            })?,
        )?;
        raw.set(
            "modified",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().modified()))?,
        )?;
        raw.set(
            "set_modified",
            lua.create_function(|lua, b: bool| {
                editor_ref(lua)?.buf_mut().set_modified(b);
                Ok(())
            })?,
        )?;
        raw.set(
            "name",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().name().to_string()))?,
        )?;
        raw.set(
            "path",
            lua.create_function(|lua, ()| {
                Ok(editor_ref(lua)?
                    .buf()
                    .path()
                    .map(|p| p.display().to_string()))
            })?,
        )?;
        raw.set(
            "id",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.selected_buffer_id()))?,
        )?;
        raw.set(
            "mode",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().mode().name.clone()))?,
        )?;
        raw.set(
            "in_comment_or_string",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.point_in_comment_or_string()))?,
        )?;

        // -- buffer management ----------------------------------------------
        raw.set(
            "buffer_ids",
            lua.create_function(|lua, ()| {
                Ok(editor_ref(lua)?
                    .buffers()
                    .iter()
                    .map(|b| b.id)
                    .collect::<Vec<usize>>())
            })?,
        )?;
        raw.set(
            "buffer_info",
            lua.create_function(|lua, id: usize| {
                let ed = editor_ref(lua)?;
                let idx = ed.buffer_index(id);
                let b = &ed.buffers()[idx];
                let t = lua.create_table()?;
                t.set("id", id)?;
                t.set("name", b.name().to_string())?;
                t.set("path", b.path().map(|p| p.display().to_string()))?;
                t.set("modified", b.modified())?;
                t.set("read_only", b.read_only())?;
                Ok(t)
            })?,
        )?;
        raw.set(
            "select_buffer",
            lua.create_function(|lua, id: usize| {
                editor_ref(lua)?.set_selected_buffer(id);
                Ok(())
            })?,
        )?;
        raw.set(
            "new_buffer",
            lua.create_function(|lua, name: String| Ok(editor_ref(lua)?.new_buffer(&name)))?,
        )?;
        raw.set(
            "kill_buffer",
            lua.create_function(|lua, id: usize| {
                editor_ref(lua)?.kill_buffer_at(id);
                Ok(())
            })?,
        )?;
        raw.set(
            "find_buffer_by_path",
            lua.create_function(|lua, path: String| {
                let ed = editor_ref(lua)?;
                Ok(ed
                    .find_buffer_by_path(Path::new(&path))
                    .map(|idx| ed.buffers()[idx].id))
            })?,
        )?;
        raw.set(
            "open_file",
            lua.create_function(|lua, path: String| {
                let buf = emacs_core::buffer::Buffer::load_file(&path)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
                let ed = editor_ref(lua)?;
                let id = buf.id;
                ed.add_buffer(buf);
                ed.set_selected_buffer(id);
                Ok(id)
            })?,
        )?;
        raw.set(
            "set_buffer_name",
            lua.create_function(|lua, (id, name): (usize, String)| {
                let ed = editor_ref(lua)?;
                let idx = ed.buffer_index(id);
                ed.buffers_mut()[idx].set_name(name);
                Ok(())
            })?,
        )?;
        raw.set(
            "set_buffer_path",
            lua.create_function(|lua, (id, path): (usize, String)| {
                let ed = editor_ref(lua)?;
                let idx = ed.buffer_index(id);
                ed.buffers_mut()[idx].set_path(Some(PathBuf::from(path)));
                Ok(())
            })?,
        )?;
        raw.set(
            "set_buffer_modified",
            lua.create_function(|lua, (id, m): (usize, bool)| {
                let ed = editor_ref(lua)?;
                let idx = ed.buffer_index(id);
                ed.buffers_mut()[idx].set_modified(m);
                Ok(())
            })?,
        )?;
        raw.set(
            "set_buffer_read_only",
            lua.create_function(|lua, (id, ro): (usize, bool)| {
                let ed = editor_ref(lua)?;
                let idx = ed.buffer_index(id);
                ed.buffers_mut()[idx].set_read_only(ro);
                Ok(())
            })?,
        )?;
        raw.set(
            "save_buffer_to_disk",
            lua.create_function(|lua, id: usize| {
                editor_ref(lua)?
                    .save_buffer_to_disk(id)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })?,
        )?;
        raw.set(
            "replace_buffer_content",
            lua.create_function(|lua, text: String| {
                editor_ref(lua)?.replace_buffer_content(&text);
                Ok(())
            })?,
        )?;

        // -- windows --------------------------------------------------------
        raw.set(
            "split_window_below",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.split_window(emacs_core::window::Split::Vertical);
                Ok(())
            })?,
        )?;
        raw.set(
            "split_window_right",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.split_window(emacs_core::window::Split::Horizontal);
                Ok(())
            })?,
        )?;
        raw.set(
            "delete_window",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.delete_window()))?,
        )?;
        raw.set(
            "delete_other_windows",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.delete_other_windows();
                Ok(())
            })?,
        )?;
        raw.set(
            "other_window",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.other_window()))?,
        )?;
        raw.set(
            "single_window",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.single_window()))?,
        )?;
        raw.set(
            "selected_window_height",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.selected_window_height()))?,
        )?;
        raw.set(
            "page_down",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.page_down_current();
                Ok(())
            })?,
        )?;
        raw.set(
            "page_up",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.page_up_current();
                Ok(())
            })?,
        )?;
        raw.set(
            "recenter",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.recenter_current();
                Ok(())
            })?,
        )?;

        // -- keymaps --------------------------------------------------------
        raw.set(
            "bind",
            lua.create_function(|lua, (seq, cmd): (String, String)| {
                let keys =
                    emacs_core::key::parse_sequence(&seq).map_err(mlua::Error::RuntimeError)?;
                editor_ref(lua)?.keymap_mut().bind_sequence(&keys, &cmd);
                Ok(())
            })?,
        )?;
        raw.set(
            "local_set_key",
            lua.create_function(|lua, (seq, cmd): (String, String)| {
                let keys =
                    emacs_core::key::parse_sequence(&seq).map_err(mlua::Error::RuntimeError)?;
                let ed = editor_ref(lua)?;
                let idx = ed.selected_buffer_index();
                ed.local_set_key(idx, &keys, &cmd);
                Ok(())
            })?,
        )?;
        raw.set(
            "lookup_key",
            lua.create_function(|lua, seq: String| {
                let keys =
                    emacs_core::key::parse_sequence(&seq).map_err(mlua::Error::RuntimeError)?;
                match editor_ref(lua)?.lookup_key(&keys) {
                    Lookup::Command(name) => Ok(("command", Some(name))),
                    Lookup::Prefix => Ok(("prefix", None)),
                    Lookup::Unbound => Ok(("unbound", None)),
                }
            })?,
        )?;
        raw.set(
            "global_bindings",
            lua.create_function(|lua, ()| {
                let bindings = editor_ref(lua)?.global_bindings();
                let list = lua.create_table()?;
                for (i, (seq, cmd)) in bindings.iter().enumerate() {
                    let t = lua.create_table()?;
                    t.set("seq", key_seq_str(seq))?;
                    t.set("cmd", cmd.clone())?;
                    list.set(i + 1, t)?;
                }
                Ok(list)
            })?,
        )?;
        raw.set(
            "local_bindings",
            lua.create_function(|lua, ()| {
                let bindings = editor_ref(lua)?.local_bindings();
                let list = lua.create_table()?;
                for (i, (seq, cmd)) in bindings.iter().enumerate() {
                    let t = lua.create_table()?;
                    t.set("seq", key_seq_str(seq))?;
                    t.set("cmd", cmd.clone())?;
                    list.set(i + 1, t)?;
                }
                Ok(list)
            })?,
        )?;
        raw.set(
            "minor_bindings",
            lua.create_function(|lua, ()| {
                let sets = editor_ref(lua)?.minor_binding_sets();
                let list = lua.create_table()?;
                for (i, (name, lighter, entries)) in sets.iter().enumerate() {
                    let t = lua.create_table()?;
                    t.set("name", name.clone())?;
                    t.set("lighter", lighter.clone())?;
                    let el = lua.create_table()?;
                    for (j, (seq, cmd)) in entries.iter().enumerate() {
                        let e = lua.create_table()?;
                        e.set("seq", key_seq_str(seq))?;
                        e.set("cmd", cmd.clone())?;
                        el.set(j + 1, e)?;
                    }
                    t.set("entries", el)?;
                    list.set(i + 1, t)?;
                }
                Ok(list)
            })?,
        )?;

        // -- modes ----------------------------------------------------------
        raw.set(
            "register_mode_def",
            lua.create_function(
                |lua, (name, lang, keymap): (String, Option<String>, Option<Table>)| {
                    let lang = match lang.as_deref() {
                        Some("rust") => Some(Lang::Rust),
                        Some("lua") => Some(Lang::Lua),
                        Some("cpp") => Some(Lang::Cpp),
                        _ => None,
                    };
                    let def = emacs_core::mode::ModeDef {
                        name: name.clone(),
                        lang,
                        keymap: parse_keymap_table(lua, keymap)?,
                    };
                    editor_ref(lua)?.register_mode_def(def);
                    Ok(())
                },
            )?,
        )?;
        raw.set(
            "register_minor_def",
            lua.create_function(
                |lua, (name, lighter, keymap): (String, String, Option<Table>)| {
                    let def = emacs_core::minor::MinorModeDef {
                        name: name.clone(),
                        doc: String::new(),
                        lighter,
                        keymap: parse_keymap_table(lua, keymap)?,
                    };
                    editor_ref(lua)?.register_minor_def(def);
                    Ok(())
                },
            )?,
        )?;
        raw.set(
            "set_buffer_mode",
            lua.create_function(|lua, name: String| {
                let ed = editor_ref(lua)?;
                let idx = ed.selected_buffer_index();
                ed.set_buffer_mode_by_name(idx, &name)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })?,
        )?;
        raw.set(
            "minor_mode_enable",
            lua.create_function(|lua, name: String| {
                let ed = editor_ref(lua)?;
                let idx = ed.selected_buffer_index();
                ed.set_minor_mode(idx, &name, true)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })?,
        )?;
        raw.set(
            "minor_mode_disable",
            lua.create_function(|lua, name: String| {
                let ed = editor_ref(lua)?;
                let idx = ed.selected_buffer_index();
                ed.set_minor_mode(idx, &name, false)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })?,
        )?;
        raw.set(
            "minor_mode_toggle",
            lua.create_function(|lua, name: String| {
                let ed = editor_ref(lua)?;
                let idx = ed.selected_buffer_index();
                ed.toggle_minor_mode(idx, &name)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })?,
        )?;
        raw.set(
            "minor_mode_enabled",
            lua.create_function(|lua, name: String| {
                let ed = editor_ref(lua)?;
                let idx = ed.selected_buffer_index();
                Ok(ed.minor_mode_enabled(idx, &name))
            })?,
        )?;

        // -- echo / misc ----------------------------------------------------
        raw.set(
            "message",
            lua.create_function(|lua, msg: String| {
                editor_ref(lua)?.message(msg);
                Ok(())
            })?,
        )?;
        raw.set(
            "error",
            lua.create_function(|lua, msg: String| {
                editor_ref(lua)?.error(msg);
                Ok(())
            })?,
        )?;
        raw.set(
            "set_quit",
            lua.create_function(|lua, q: bool| {
                editor_ref(lua)?.set_quit(q);
                Ok(())
            })?,
        )?;
        raw.set(
            "replay_key",
            lua.create_function(|lua, ()| {
                editor_ref(lua)?.set_replay();
                Ok(())
            })?,
        )?;
        raw.set(
            "execute",
            lua.create_function(|lua, name: String| {
                run_command_fn(lua)?.call::<()>(name)?;
                Ok(())
            })?,
        )?;
        raw.set(
            "cwd",
            lua.create_function(|_lua, ()| {
                Ok(std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("/"))
                    .display()
                    .to_string())
            })?,
        )?;
        raw.set(
            "expand_tilde",
            lua.create_function(|_lua, input: String| {
                Ok(if let Some(rest) = input.strip_prefix("~/") {
                    if let Some(home) = std::env::var_os("HOME") {
                        format!("{}/{}", Path::new(&home).display(), rest)
                    } else {
                        input
                    }
                } else {
                    input
                })
            })?,
        )?;
        raw.set(
            "canonicalize",
            lua.create_function(|_lua, path: String| {
                Ok(Path::new(&path)
                    .canonicalize()
                    .ok()
                    .map(|p| p.display().to_string()))
            })?,
        )?;
        raw.set(
            "is_dir",
            lua.create_function(|_lua, path: String| Ok(Path::new(&path).is_dir()))?,
        )?;
        raw.set(
            "read_dir",
            lua.create_function(|lua, path: String| {
                let list = lua.create_table()?;
                let rd = std::fs::read_dir(&path)
                    .map_err(|e| mlua::Error::RuntimeError(format!("cannot read {path}: {e}")))?;
                let mut i = 0usize;
                for entry in rd {
                    let Ok(entry) = entry else { continue };
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let md = entry
                        .metadata()
                        .map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;
                    let t = lua.create_table()?;
                    t.set("name", name)?;
                    t.set("is_dir", md.is_dir())?;
                    t.set("size", md.len())?;
                    i += 1;
                    list.set(i, t)?;
                }
                Ok(list)
            })?,
        )?;
        raw.set(
            "delete_file",
            lua.create_function(|_lua, (path, recursive): (String, bool)| {
                let r = if recursive {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                r.map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })?,
        )?;
        raw.set(
            "rename_file",
            lua.create_function(|_lua, (src, dest): (String, String)| {
                std::fs::rename(&src, &dest).map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })?,
        )?;
        raw.set(
            "copy_file",
            lua.create_function(|_lua, (src, dest): (String, String)| {
                copy_recursive(Path::new(&src), Path::new(&dest))
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })?,
        )?;
        raw.set(
            "mkdir",
            lua.create_function(|_lua, path: String| {
                std::fs::create_dir(&path).map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })?,
        )?;

        globals.set("raw", raw)?;
        Ok(())
    }
}

fn key_seq_str(seq: &[Key]) -> String {
    seq.iter()
        .map(|k| k.to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn copy_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    let md = std::fs::metadata(src)?;
    if md.is_dir() {
        std::fs::create_dir_all(dest)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dest.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(src, dest).map(|_| ())
    }
}

impl ScriptHost for LuaHost {
    fn load_file(&mut self, path: &Path, editor: &mut Editor) -> Result<()> {
        let code = std::fs::read_to_string(path)?;
        let name = path.display().to_string();
        with_editor(&self.lua, editor, |lua| {
            lua.load(&code).set_name(&name).exec()?;
            Ok(())
        })
    }

    /// Load the runtime modules from `dir`, in the hardcoded order. A
    /// missing module is a fatal error; a module that fails to execute or
    /// a runtime that never installs its entry points is fatal too.
    fn load_runtime(&mut self, dir: &Path, editor: &mut Editor) -> Result<()> {
        for name in RUNTIME_MODULES {
            let path = dir.join(name);
            if !path.is_file() {
                return Err(anyhow!(
                    "required Lua runtime module is missing: {}",
                    path.display()
                ));
            }
            self.load_file(&path, editor)?;
        }
        with_editor(&self.lua, editor, |lua| {
            internals_table(lua)?;
            run_command_fn(lua)?;
            startup_fn(lua)?;
            Ok(())
        })
    }

    fn call_command(
        &mut self,
        name: &str,
        extra: Option<char>,
        editor: &mut Editor,
    ) -> Result<CommandOutcome> {
        self.start_command(name, extra, editor)
    }

    fn resume_pending(
        &mut self,
        value: ResumeValue,
        editor: &mut Editor,
    ) -> Result<CommandOutcome> {
        self.resume_coroutine(value, editor)
    }

    fn update_completion(&mut self, input: &str, editor: &mut Editor) -> Result<Vec<String>> {
        let f = self.pending.as_ref().and_then(|p| p.completion.clone());
        let Some(f) = f else {
            return Ok(Vec::new());
        };
        with_editor(&self.lua, editor, |_lua| f.call(input))
    }

    fn run_startup(&mut self, path: Option<&str>, editor: &mut Editor) -> Result<()> {
        with_editor(&self.lua, editor, |lua| {
            let f = startup_fn(lua)?;
            match path {
                Some(p) => f.call::<()>(p)?,
                None => f.call::<()>(Option::<String>::None)?,
            }
            Ok(())
        })
    }
}
