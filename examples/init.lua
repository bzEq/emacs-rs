-- -*- mode: lua -*-
-- emacs-rs example configuration.
-- Copy this to ~/.config/emacs-rs/init.lua and edit to taste.
--
-- emacs-rs is Lua-first: the editor core (ropes, windows, rendering) is a
-- small Rust binary, and everything else — commands, keybindings, modes,
-- undo, the kill ring, isearch, dired — is Lua. The defaults live in the
-- runtime `lua/` directory and load before this file, so anything here
-- overrides or extends them.
--
-- Commands run as coroutines, so `emacs.read_string` / `emacs.read_key`
-- suspend synchronously until the input arrives.

---------------------------------------------------------------------
-- 1. Commands: define a new interactive command, then bind it.
---------------------------------------------------------------------

-- Commands receive the numeric prefix argument (C-u, C-3, M--).
emacs.define_command("insert-timestamp", function(prefix)
  emacs.insert("T" .. tostring(prefix))
end)

-- Global key bindings (Emacs key syntax: "C-x C-f", "M-f", "C-c t", ...).
emacs.bind("C-c t", "insert-timestamp")

-- Built-in commands can be rebound too.
-- emacs.bind("C-z", "undo")

---------------------------------------------------------------------
-- 2. Synchronous minibuffer reads (coroutine-based).
---------------------------------------------------------------------

emacs.define_command("my-ask", function()
  local name = emacs.read_string("Your name: ", nil)
  if name then
    emacs.message("hello, " .. name)
  end
end)

-- emacs.read_string(prompt, completion_fn)  -- completion_fn(input) -> names
-- emacs.read_yes_no(prompt)                 -- -> true / false / nil (C-g)
-- emacs.read_key()                          -- -> "C-x", "RET", "a", ...

---------------------------------------------------------------------
-- 3. Major modes: per-buffer indentation and local keymap.
---------------------------------------------------------------------

-- A Lua-defined major mode. `indent` is the indentation unit in spaces.
-- The keymap is active only in buffers using this mode, and it shadows
-- the global keymap.
emacs.define_major_mode("txt-mode", {
  indent = 2,
  keymap = {
    ["C-c h"] = "insert-timestamp",   -- local binding example
  },
})

-- Switch the current buffer to a major mode by name (also M-x txt-mode).
-- emacs.set_buffer_mode("txt-mode")

-- You can also redefine a built-in mode:
-- emacs.define_major_mode("rust-mode", {
--   indent = 2,
--   keymap = { ["C-c c"] = "insert-timestamp" },
-- })

---------------------------------------------------------------------
-- 4. Minor modes: per-buffer toggles with an optional keymap.
---------------------------------------------------------------------

-- Defines a toggle command `my-extra-mode` (M-x my-extra-mode) and shows
-- the lighter "XX" in the modeline while enabled. Keymaps of enabled
-- minor modes override local and global bindings.
emacs.define_minor_mode("my-extra", {
  lighter = "XX",
  doc = "Demonstration minor mode.",
  keymap = {
    ["C-c e"] = "insert-timestamp",
  },
})

-- Toggle directly from Lua:
-- emacs.minor_mode_enable("my-extra")
-- emacs.minor_mode_disable("my-extra")
-- emacs.minor_mode_toggle("my-extra")

-- Built-in minor mode: line numbers in the gutter (M-x line-numbers-mode).
-- emacs.minor_mode_enable("line-numbers")

---------------------------------------------------------------------
-- 5. Hooks.
---------------------------------------------------------------------

emacs.add_hook("before_save", function()
  emacs.message("saving " .. tostring(emacs.buffer_name()) .. "...")
end)

---------------------------------------------------------------------
-- 6. Buffer editing API (all operate on the current buffer).
---------------------------------------------------------------------

-- emacs.insert("text")              insert at point (records undo)
-- emacs.newline()                   insert a newline
-- emacs.delete_backward()           backspace (records undo)
-- emacs.delete_forward()            delete (records undo)
-- emacs.delete_range(start, end)    delete a range (records undo)
-- emacs.point() -> n                char offset of point
-- emacs.set_point(n)
-- emacs.get_text(start, end) -> str
-- emacs.buffer_string() -> str
-- emacs.buffer_name() -> str
-- emacs.buffer_path() -> str|nil
-- emacs.save_buffer()               save the current buffer
-- emacs.execute("command-name")     run a command by name
-- emacs.kill("text")                append text to the kill ring
-- emacs.yank()                      insert the current kill
-- emacs.message("msg") / emacs.error("msg")

-- emacs.bind(seq, cmd)              global binding
-- emacs.local_set_key(seq, cmd)     binding in the current buffer only
-- emacs.bind_isearch(seq, cmd)      binding while isearch (C-s/C-r) is active
-- emacs.define_command(name, fn)    new command (fn receives prefix arg)
-- emacs.add_hook(name, fn)          hooks: before_save, after_save

-- The `raw` table exposes the full set of Rust primitives (rope motion,
-- search, buffer/window management, filesystem) that the defaults are
-- built from; most init files never need it.

-- Note: this LuaJIT does not support the `|` alternation operator in
-- string patterns (a LuaJIT limitation) — chain several `:match` calls
-- instead.
