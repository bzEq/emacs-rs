# emacs-rs

An Emacs-like text editor with a minimal Rust core and LuaJIT for
everything else — the way Emacs splits a small C core from Emacs Lisp.

## Architecture

```
Rust core (crates/)               Lua runtime (lua/, loaded at startup)
------------------------------    --------------------------------------
rope buffer primitives            all commands (motion, editing, ...)
window tree / scrolling           undo & kill ring
key parsing / keymap lookup       prefix arguments (C-u, C-3, M--)
rendering (ratatui)               isearch (C-s / C-r)
terminal event loop               dired
tree-sitter highlighting          major / minor modes & keybindings
minibuffer input editing          completion functions
filesystem & buffer primitives    find-file, save, buffers, quit
                                  M-x, describe-key/bindings
```

Commands are Lua functions driven as **coroutines**: a command that needs
input (minibuffer string, yes/no, a raw key) suspends with
`coroutine.yield` and the Rust event loop resumes it when the input
arrives, so Lua commands use plain synchronous reads:

```lua
emacs.define_command("ask", function()
  local name = emacs.read_string("Name: ", nil)
  local ok = emacs.read_yes_no("Continue? (y/n)")
  if name and ok then emacs.insert("hi " .. name) end
end)
```

The Rust binary only knows how to: read keys, look them up in keymaps,
render the state, and shuttle input between the terminal and suspended
Lua coroutines. All policy — what commands exist, what keys run them,
what modes do, how undo and the kill ring behave — lives in the `lua/`
directory, so users can redefine any part of the editor from their
`init.lua`.

## Features

- **Rope buffer**: `ropey`-backed with O(log n) edits; a 100MB log file
  opens in ~70ms (~1.5GB/s), and 1GB files stay responsive
- **Emacs editing experience**
  - Classic keybindings: motion/editing/kill-ring/undo/mark, `C-x` prefix
    keys, Esc as Meta
  - Command system: `M-x` runs any command by name, with automatic
    completion (common prefix fills as you type, TAB cycles); the
    minibuffer pre-fills the default directory for find-file (editable
    input, Emacs-style), keeps an input history (`C-p`/`C-n` recall), and
    supports `C-f`/`C-b`/`C-a`/`C-e`/`C-d` editing; typing `/` over the
    pre-filled directory replaces it (file-name-shadow)
  - The minibuffer has its own keymap (defined in Lua, like Emacs's
    `minibuffer-local-map`) that inherits the global map: user bindings
    keep working while the minibuffer is active (`C-x C-c` quits from
    there, custom keys run normally)
  - Incremental search: `C-s` / `C-r`, case-insensitive, wraps around,
    `C-g` aborts
  - Undo (with boundaries), kill ring (consecutive kills accumulate),
    prefix arguments (`C-u`/`C-3`) — all implemented in Lua
  - CRLF files follow Emacs semantics (`\r\n` acts as a single newline)
- **Window system**: `C-x 2/3` splits, `C-x 0/1` deletes, `C-x o` cycles;
  each window keeps its own point and scroll position
- **Dired**: `C-x d` directory browser — listing, marks (m/u/U), delete
  (D), rename (R), copy (C), mkdir (+), subdirectory navigation;
  `find-file` or a directory command-line argument opens dired
  automatically
- **Syntax highlighting**: tree-sitter (Rust, Lua, C++ built in), colored
  by node type, with parse size caps and a re-parse cooldown so large
  files stay fast
- **Auto-indentation**: `RET` indents smartly (`{` indents, `}`/`end`
  outdents), `TAB` re-indents the current line, `C-j` runs
  `electric-newline-and-maybe-indent` (no indent inside comments/strings),
  Backspace at line start deletes one indent unit — all in Lua
- **Major / minor modes**: major mode chosen by file extension; minor
  modes like `line-numbers` toggle per buffer; modes can carry local
  keymaps (lighters shown in the modeline)

## Build and run

Requirements: a stable Rust toolchain and a C compiler (for the vendored
LuaJIT build).

```sh
cargo build --release
./target/release/em [--init <init.lua>] [FILE]
```

- `FILE` opens a file, or dired if it is a directory
- `--init` selects the init file; the default is
  `~/.config/emacs-rs/init.lua` (respecting `XDG_CONFIG_HOME`)
- The Lua runtime is located via `EMACS_RS_LUA_DIR`, a `lua/` directory
  next to the executable, or `../lua` relative to it (which matches the
  repo layout when running from `target/`)

## The Lua runtime

The runtime is a fixed set of modules in `lua/`. The list and load order
are hardcoded in `emacs-lua` (`RUNTIME_MODULES`); a missing or broken
module is a fatal error — the editor reports it and exits instead of
starting without a working runtime. Extra modules are not auto-loaded:
`dofile` them from `init.lua`.

| File | Contents |
|---|---|
| `api.lua` | state, undo, kill ring, prefix args, command machinery, the `emacs` API |
| `motion.lua` | motion commands |
| `editing.lua` | editing commands, auto-indentation |
| `search.lua` | incremental search |
| `windows.lua` | window commands |
| `files.lua` | find/save/write/switch/kill-buffer, file completion |
| `modes.lua` | built-in major and minor modes |
| `dired.lua` | the directory editor |
| `help.lua` | `M-x`, describe-key, describe-bindings |
| `bindings.lua` | the default global keymap |

The `emacs` module is the user-facing API; the `raw` module underneath it
exposes the Rust primitives (rope editing, motion, search, buffers,
windows, keymaps, modes, filesystem) that the defaults build on.

## Configuration (init.lua)

See the fully commented example in
[`examples/init.lua`](examples/init.lua). Core API:

```lua
-- commands and keybindings
emacs.define_command("my-cmd", function(prefix) emacs.insert("x") end)
emacs.bind("C-c x", "my-cmd")            -- global binding (overrides defaults)
emacs.local_set_key("C-c y", "my-cmd")   -- binding local to the current buffer

-- synchronous reads (coroutine-based)
local name = emacs.read_string("Name: ", nil)
local yes  = emacs.read_yes_no("Sure? (y/n)")
local key  = emacs.read_key()            -- "C-x", "RET", "a", ...

-- major / minor modes
emacs.define_major_mode("txt-mode", {
  indent = 2,
  language = "lua",                      -- optional: enables highlighting
  keymap = { ["C-c h"] = "my-cmd" },
})
emacs.define_minor_mode("my-extra", {
  lighter = "XX",
  keymap = { ["C-c e"] = "my-cmd" },
})

-- buffer operations (all act on the current buffer)
emacs.insert("text"); emacs.point(); emacs.set_point(n)
emacs.buffer_string(); emacs.save_buffer(); emacs.execute("command")

-- hooks
emacs.add_hook("before_save", function() emacs.message("saving...") end)
```

Key syntax: `C-x C-f`, `M-f`, `C-M-a`, `RET`, `TAB`, `DEL`, `SPC`,
`<left>`, `<f1>`.

Note: this LuaJIT build does not support the `|` alternation operator in
string patterns; chain several `:match` calls instead.

## Common keybindings

| Key | Command | Key | Command |
|---|---|---|---|
| `C-f/b/n/p` | char/line motion | `C-x C-f` | find file |
| `M-f/M-b` | word motion | `C-x C-s` | save buffer |
| `C-a/C-e` | line start/end | `C-x d` | dired |
| `C-k/C-w/M-w` | kill line/region / copy | `C-x b/k` | switch/kill buffer |
| `C-y/M-y` | yank / yank-pop | `C-x 2/3/0/1/o` | window operations |
| `C-/` `C-_` `C-x u` | undo | `C-s/C-r` | incremental search |
| `C-g` | cancel | `M-x` | execute command (with completion) |
| `C-u/C-3/M--` | prefix argument | `C-h k/b` | describe key/bindings |

## Testing

```sh
cargo test
```

- `crates/core` unit tests: rope buffer semantics (goal column, CRLF),
  key parsing, keymaps, window tree, minibuffer editing, search
  primitives, syntax highlighting
- `crates/lua/tests` unit tests drive the real Lua runtime against an
  in-memory editor: commands, undo, kill ring, prefix arguments, isearch
  (via coroutine resumes), indentation, dired, and the read-string /
  read-key / yes-no protocols
- `crates/app/tests` PTY integration tests: spawn the real `em` binary in
  a pseudo-terminal, send keystrokes, reconstruct the screen, and assert
  (editing, windows, search, highlighting, modes, completion, dired, CLI)

CI (GitHub Actions) runs formatting, clippy, and the full test suite on
every push to `main` and every pull request.

## License

[Apache License 2.0](LICENSE)
