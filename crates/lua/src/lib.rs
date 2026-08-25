//! LuaJIT scripting host: exposes an `emacs` module to Lua (init.lua) and
//! executes Lua-defined commands and hooks.
//!
//! # Safety of the editor reference
//!
//! Lua code runs only on the main thread, inside `ScriptHost` methods that
//! receive `&mut Editor`. Each such call stores a raw pointer to the editor
//! in Lua app-data for the duration of the call and removes it afterwards,
//! so the `emacs.*` API can reach the editor from Lua callbacks. During Lua
//! execution the surrounding `&mut Editor` is not accessed, so no aliasing
//! occurs. The editor must not move while a call is in flight (it is owned
//! by the app's main function, so this holds).

use std::path::Path;

use anyhow::{anyhow, Result};
use mlua::{Function, Lua, Table};

use emacs_core::editor::Editor;
use emacs_core::script::ScriptHost;

struct EditorRef(*mut Editor);

unsafe impl Send for EditorRef {}

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

pub struct LuaHost {
    lua: Lua,
}

impl LuaHost {
    pub fn new() -> Result<Self> {
        let lua = Lua::new();
        let host = LuaHost { lua };
        host.install_api().map_err(anyhow_err)?;
        Ok(host)
    }

    /// Run `f` with the editor reachable from Lua callbacks. mlua errors are
    /// converted to anyhow at this boundary (mlua::Error is not Send/Sync).
    fn with_editor<R>(
        &mut self,
        editor: &mut Editor,
        f: impl FnOnce(&Lua) -> mlua::Result<R>,
    ) -> Result<R> {
        self.lua
            .set_app_data::<Option<EditorRef>>(Some(EditorRef(editor)));
        let r = f(&self.lua).map_err(anyhow_err);
        self.lua.set_app_data::<Option<EditorRef>>(None);
        r
    }

    fn install_api(&self) -> mlua::Result<()> {
        let lua = &self.lua;
        let globals = lua.globals();
        let emacs = lua.create_table()?;
        emacs.set("_commands", lua.create_table()?)?;
        emacs.set("_hooks", lua.create_table()?)?;
        emacs.set("_next_id", 0u32)?;

        // --- buffer editing -------------------------------------------------
        emacs.set(
            "insert",
            lua.create_function(|lua, text: String| {
                let ed = editor_ref(lua)?;
                ed.buf_mut().insert(&text);
                Ok(())
            })?,
        )?;
        emacs.set(
            "newline",
            lua.create_function(|lua, ()| {
                let ed = editor_ref(lua)?;
                ed.buf_mut().insert("\n");
                Ok(())
            })?,
        )?;
        emacs.set(
            "delete_backward",
            lua.create_function(|lua, ()| {
                let ed = editor_ref(lua)?;
                ed.buf_mut().delete_backward();
                Ok(())
            })?,
        )?;
        emacs.set(
            "delete_forward",
            lua.create_function(|lua, ()| {
                let ed = editor_ref(lua)?;
                ed.buf_mut().delete_forward();
                Ok(())
            })?,
        )?;
        emacs.set(
            "point",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().point()))?,
        )?;
        emacs.set(
            "set_point",
            lua.create_function(|lua, n: usize| {
                let ed = editor_ref(lua)?;
                ed.buf_mut().set_point(n);
                Ok(())
            })?,
        )?;
        emacs.set(
            "get_text",
            lua.create_function(|lua, (start, end): (usize, usize)| {
                let ed = editor_ref(lua)?;
                let rope = ed.buf().rope();
                let start = start.min(rope.len_chars());
                let end = end.min(rope.len_chars());
                Ok(rope.slice(start.min(end)..start.max(end)).to_string())
            })?,
        )?;
        emacs.set(
            "buffer_string",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().rope().to_string()))?,
        )?;
        emacs.set(
            "buffer_name",
            lua.create_function(|lua, ()| Ok(editor_ref(lua)?.buf().name().to_string()))?,
        )?;
        emacs.set(
            "buffer_path",
            lua.create_function(|lua, ()| {
                Ok(editor_ref(lua)?
                    .buf()
                    .path()
                    .map(|p| p.display().to_string()))
            })?,
        )?;

        // --- commands / messages -------------------------------------------
        emacs.set(
            "message",
            lua.create_function(|lua, msg: String| {
                editor_ref(lua)?.message(msg);
                Ok(())
            })?,
        )?;
        emacs.set(
            "error",
            lua.create_function(|lua, msg: String| {
                editor_ref(lua)?.error(msg);
                Ok(())
            })?,
        )?;
        emacs.set(
            "save_buffer",
            lua.create_function(|lua, ()| {
                let ed = editor_ref(lua)?;
                let idx = ed.selected_buffer_index();
                ed.save_buffer_now(idx)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })?,
        )?;
        emacs.set(
            "execute",
            lua.create_function(|lua, name: String| {
                let ed = editor_ref(lua)?;
                ed.invoke_command(&name)
                    .map_err(|e| mlua::Error::RuntimeError(e.to_string()))
            })?,
        )?;
        emacs.set(
            "kill",
            lua.create_function(|lua, text: String| {
                editor_ref(lua)?.kill(text);
                Ok(())
            })?,
        )?;
        emacs.set(
            "yank",
            lua.create_function(|lua, ()| {
                let ed = editor_ref(lua)?;
                let text = ed
                    .kill_ring()
                    .current()
                    .map(|s| s.to_string())
                    .ok_or_else(|| mlua::Error::RuntimeError("kill ring is empty".into()))?;
                ed.buf_mut().insert(&text);
                Ok(())
            })?,
        )?;

        // --- defining commands, bindings, hooks ----------------------------
        emacs.set(
            "define_command",
            lua.create_function(|lua, (name, f): (String, Function)| {
                let globals = lua.globals();
                let emacs: Table = globals.get("emacs")?;
                let commands: Table = emacs.get("_commands")?;
                let next: u32 = emacs.get("_next_id")?;
                emacs.set("_next_id", next + 1)?;
                commands.set(next, f)?;
                let ed = editor_ref(lua)?;
                ed.commands_mut().add_lua(&name, next);
                Ok(())
            })?,
        )?;
        emacs.set(
            "bind",
            lua.create_function(|lua, (seq, cmd): (String, String)| {
                let keys = emacs_core::key::parse_sequence(&seq)
                    .map_err(|e| mlua::Error::RuntimeError(e))?;
                let ed = editor_ref(lua)?;
                ed.keymap_mut().bind_sequence(&keys, &cmd);
                Ok(())
            })?,
        )?;
        emacs.set(
            "add_hook",
            lua.create_function(|lua, (name, f): (String, Function)| {
                let globals = lua.globals();
                let emacs: Table = globals.get("emacs")?;
                let hooks: Table = emacs.get("_hooks")?;
                let list: Table = match hooks.get::<Table>(name.as_str()) {
                    Ok(t) => t,
                    Err(_) => {
                        let t = lua.create_table()?;
                        hooks.set(name.as_str(), t.clone())?;
                        t
                    }
                };
                list.set(list.raw_len() + 1, f)?;
                Ok(())
            })?,
        )?;

        globals.set("emacs", emacs)?;
        Ok(())
    }
}

impl ScriptHost for LuaHost {
    fn call_command(&mut self, id: u32, editor: &mut Editor) -> Result<()> {
        let prefix = editor.prefix_arg().value();
        self.with_editor(editor, |lua| {
            let emacs: Table = lua.globals().get("emacs")?;
            let commands: Table = emacs.get("_commands")?;
            let f: Function = commands
                .get(id)
                .map_err(|e| mlua::Error::RuntimeError(format!("lua command {id} missing: {e}")))?;
            f.call::<()>(prefix)?;
            Ok(())
        })
    }

    fn call_hook(&mut self, name: &str, editor: &mut Editor) -> Result<()> {
        let name = name.to_string();
        self.with_editor(editor, |lua| {
            let emacs: Table = lua.globals().get("emacs")?;
            let hooks: Table = emacs.get("_hooks")?;
            if let Ok(list) = hooks.get::<Table>(name.as_str()) {
                for i in 1..=list.raw_len() {
                    let f: Function = list.get(i)?;
                    if let Err(e) = f.call::<()>(()) {
                        return Err(mlua::Error::RuntimeError(format!(
                            "hook {name} failed: {e}"
                        )));
                    }
                }
            }
            Ok(())
        })
    }

    fn load_file(&mut self, path: &Path, editor: &mut Editor) -> Result<()> {
        let code = std::fs::read_to_string(path)?;
        let name = path.display().to_string();
        self.with_editor(editor, |lua| {
            lua.load(&code).set_name(&name).exec()?;
            Ok(())
        })
    }
}
