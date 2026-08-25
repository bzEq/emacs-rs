# emacs-rs

An Emacs-like text editor written in Rust: rope-backed buffers for
large-file performance, Emacs keybindings and command system, and LuaJIT
as the extension language.

## Features

- **Rope buffer**: `ropey`-backed with O(log n) edits; a 100MB log file
  opens in ~70ms (~1.5GB/s), and 1GB files stay responsive
- **Emacs editing experience**
  - Classic keybindings: motion/editing/kill-ring/undo/mark, `C-x` prefix
    keys, Esc as Meta
  - Command system: `M-x` runs any command by name, with automatic
    completion (common prefix fills as you type, TAB cycles)
  - Incremental search: `C-s` / `C-r`, case-insensitive, wraps around,
    `C-g` aborts
  - Undo (with boundaries), kill ring (consecutive kills accumulate),
    prefix arguments (`C-u`/`C-3`)
  - CRLF files follow Emacs semantics (`\r\n` acts as a single newline)
- **Window system**: `C-x 2/3` splits, `C-x 0/1` deletes, `C-x o` cycles;
  each window keeps its own point and scroll position
- **Dired**: `C-x d` directory browser — listing, marks (m/u/U), delete
  (D), rename (R), copy (C), mkdir (+), subdirectory navigation;
  `find-file` or a directory command-line argument opens dired
  automatically
- **Syntax highlighting**: tree-sitter (Rust and Lua built in), colored by
  node type, with parse size caps and a re-parse cooldown so large files
  stay fast
- **Auto-indentation**: `RET` indents smartly (`{` indents, `}`/`end`
  outdents), `TAB` re-indents the current line, `C-j` runs
  `electric-newline-and-maybe-indent` (no indent inside comments/strings),
  Backspace at line start deletes one indent unit
- **Major / minor modes**: major mode chosen by file extension; minor
  modes like `line-numbers` toggle per buffer; modes can carry local
  keymaps (lighters shown in the modeline)
- **LuaJIT extensions**: `mlua` with vendored LuaJIT (no system
  dependency); `init.lua` can define commands, bind keys, define
  major/minor modes, and register hooks

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

## Configuration (init.lua)

See the fully commented example in
[`examples/init.lua`](examples/init.lua). Core API:

```lua
-- commands and keybindings
emacs.define_command("my-cmd", function(prefix) emacs.insert("x") end)
emacs.bind("C-c x", "my-cmd")            -- global binding (overrides defaults)
emacs.local_set_key("C-c y", "my-cmd")   -- binding local to the current buffer

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

## Architecture

```
crates/
  core/   # pure logic: rope buffer, undo, kill ring, keymap, command
          # system, window tree, isearch, dired, indentation,
          # tree-sitter highlighting
  lua/    # mlua + LuaJIT: the emacs module (commands/keybindings/modes/hooks)
  ui/     # ratatui rendering: window tree, modeline, echo area,
          # completion preview
  app/    # the em binary: event loop (read -> execute -> render), CLI
```

## Testing

```sh
cargo test
```

- `crates/core` unit tests: buffer semantics (goal column, CRLF), undo,
  kill ring, keymaps, window tree, isearch, indentation, syntax
  highlighting, dired
- `crates/app/tests` PTY integration tests: spawn the real `em` binary in
  a pseudo-terminal, send keystrokes, reconstruct the screen, and assert
  (editing, windows, search, highlighting, modes, completion, dired, CLI)

CI (GitHub Actions) runs formatting, clippy, and the full test suite on
every push to `main` and every pull request.

## License

[Apache License 2.0](LICENSE)
