# emacs-rs

An Emacs-like text editor with a minimal Rust core and LuaJIT for
everything else — the way Emacs splits a small C core from Emacs Lisp.

Scope: a small, fast **plain-text** editor. emacs-rs borrows Emacs's
editing model (buffers, windows, modes, keymaps, minibuffer) but not its
breadth: binary/unibyte buffers, text properties/overlays, and other
heavyweight subsystems are intentionally out of scope. The Rust core stays
minimal; policy and optional features live in Lua extensions.

## Architecture

```
Rust core (crates/)               Lua runtime (lua/, loaded at startup)
------------------------------    --------------------------------------
rope buffer primitives            all commands (motion, editing, ...)
window tree / scrolling           undo & kill ring
key parsing / keymap lookup       prefix arguments (C-u, C-3, M--)
rendering (ratatui)               isearch (C-s / C-r)
terminal event loop               dired
file watching (inotify)           major / minor modes & keybindings
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

### Crates

| Crate | Role |
|---|---|
| `emacs-core` | The minimal Rust core: text/window/key/mode/input *mechanisms* plus file watching, with no scripting dependency |
| `emacs-lua` | LuaJIT host: exposes the core primitives as the `raw` API, embeds the `lua/` runtime, drives commands as coroutines |
| `emacs-ui` | Rendering: turns `Editor` state into the ratatui screen (window tree, modeline, echo area / minibuffer) |
| `emacs-app` (`em`) | Entry point, terminal setup, the event loop, and the terminal key adapter (`to_key`); owns the `FileWatcher` instance and renders via `emacs-ui` |

### Core components (`crates/core`)

| Component | Role | Deliberately not |
|---|---|---|
| `buffer.rs` | The single concrete `Buffer`: rope text + point/mark/goal column + name/path/modified/read-only + major mode, local keymap, minor modes; motion/editing/line queries/file IO | Undo, kill ring, command policy |
| `editor.rs` | The core coordinator and world state: `buffers`, window tree, global/overriding keymaps, minibuffer state and history, echo, pending request/keys, mode registries, script host, current-buffer resolution, applying command outcomes | Concrete command semantics |
| `input.rs` | The keyboard/command loop (`keyboard.c`-lite): routes one key into a pending read request or keymap dispatch, with replay/error handling and minibuffer completion upkeep | What commands do |
| `key.rs` | Terminal-independent key model (`Key`/`KeyCode`/`Modifiers`) and Emacs-style sequence parsing/display | Bindings and dispatch |
| `keymap.rs` | Sparse prefix keymap; `lookup → Command/Prefix/Unbound`; `flatten` for describe-bindings | What is bound to what (Lua decides) |
| `mode.rs` / `minor.rs` | Mode *definitions* (name + keymap; the active major mode also carries its `Mode`) | Indentation, completion, mode behavior (in Lua) |
| `minibuffer.rs` | State around the input line: prompt, completion flag, candidates, preview/cycle, history index, `buffer_id`; edits take a `&mut Buffer` | The input text itself (it is a real registered `Buffer`) |
| `script.rs` | Dependency inversion: the `ScriptHost` trait plus `CommandOutcome`/`PendingRequest`/`ResumeValue`; the core knows nothing about Lua | Lua details |
| `window.rs` | Window tree: leaves display buffer ids, interior nodes split; select/split/delete/cycle, layout, dividers | Scrolling and wrapping math |
| `view.rs` | Per-window scroll state (first visible visual row) with caches for O(delta) row math | Text storage |
| `wrap.rs` | Visual word-wrap shared by scrolling and rendering, plus a row walker so they agree on line breaks | Drawing |
| `search.rs` | Rope search primitives used by isearch (case-insensitive substring, forward/backward), with an ASCII fast path | isearch interaction (in Lua) |
| `watch.rs` | File watching (inotify via `notify`): maps buffer files to their parent dirs and reports the buffers whose files changed | Revert policy (in Lua) |

### Design principles

1. **Mechanism vs. policy**: Rust provides primitives (rope editing,
   window tree, scrolling/wrapping, key parsing, keymap lookup, the event
   loop, the script bridge, rendering); commands, undo, the kill ring,
   isearch, dired, completion and mode behavior live in `lua/`.
2. **Commands are Lua coroutines**: a command that needs input yields a
   `PendingRequest`; the core resumes it when the input arrives, so Lua
   code reads synchronously (`emacs.read_string/read_yes_no/read_key`).
3. **Current-buffer model (Emacs's)**: during a minibuffer read the
   minibuffer is a real registered `Buffer` (with `minibuffer-mode` and a
   buffer-local keymap) and `buf()/buf_mut()` point at it; it is killed
   when the read finishes.
4. **Key lookup layering**: minor modes (most recently enabled first) →
   buffer-local (major mode) → global; isearch uses a core-level
   *overriding* keymap (Emacs's `overriding-terminal-local-map`), isolated
   from normal dispatch.
5. **One concrete `Buffer` type**: no trait hierarchy; unibyte/binary
   storage, buffer-local variables and text properties/overlays are out of
   scope for a plain-text editor.
6. **Optional features are Lua extensions**: e.g.
   `extensions/clang-format.lua`.

A keypress flows as: `app` event loop → `to_key` (terminal adapter) →
`core::input::handle_key` → (a pending read? route to its protocol) →
otherwise `Editor::lookup_key` → `ScriptHost` starts or resumes the
command coroutine → `PendingRequest` (suspend) or `Done` (possibly with a
replay key) → `emacs-ui` renders the next frame from the `Editor` state.

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
  - The minibuffer input is a real, registered buffer with its own major
    mode (`minibuffer-mode`) and buffer-local keymap, like Emacs's
    `minibuffer-local-map`: it overrides the global map for the keys it
    binds, and everything else falls through, so user bindings keep
    working while the minibuffer is active (`C-x C-c` quits from there,
    custom keys run normally). Redefine `minibuffer-mode` from init.lua
    to change its bindings.
  - Incremental search: `C-s` / `C-r`, case-insensitive, wraps around,
    `C-g` aborts; the search keys are dispatched through an overriding
    keymap (`M.isearch_bindings`, Emacs's `overriding-terminal-local-map`)
    that can be customized with `emacs.bind_isearch`
  - Undo (with boundaries), kill ring (consecutive kills accumulate),
    prefix arguments (`C-u`/`C-3`) — all implemented in Lua
  - Active region: after `C-SPC` the text between point and mark is
    highlighted (transient-mark-mode), and `C-w`/`M-w` act on it
  - global-auto-revert-mode: buffer files are watched with inotify
    (via the `notify` crate); when another program changes a file, the
    buffer reverts — or warns instead if it has unsaved edits. Manual
    `M-x revert-buffer` included; toggle with `M-x
    global-auto-revert-mode`
  - CRLF files follow Emacs semantics (`\r\n` acts as a single newline)
  - Visual line wrapping: long lines word-wrap to the window width
    (Emacs visual-line-mode); scrolling, the cursor, and the isearch
    match highlight all follow the wrapped rows
- **Window system**: `C-x 2/3` splits, `C-x 0/1` deletes, `C-x o` cycles;
  each window keeps its own point and scroll position
- **Dired**: `C-x d` directory browser — listing, marks (m/u/U), delete
  (D), rename (R), copy (C), mkdir (+), subdirectory navigation;
  `find-file` or a directory command-line argument opens dired
  automatically
- **Auto-indentation**: `RET` indents smartly (`{` indents, `}`/`end`
  outdents), `TAB` re-indents the current line, Backspace at line start
  deletes one indent unit — all in Lua
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
- The Lua runtime is embedded in the binary at compile time — a single
  `em` executable needs no files next to it. Set `EMACS_RS_LUA_DIR` to a
  directory of runtime modules to load them from disk instead (useful
  while developing the runtime; a missing or broken module is a fatal
  error)

## The Lua runtime

The runtime is a fixed set of modules in `lua/` (the source of truth in
the repo). The list and load order are hardcoded in `emacs-lua`
(`RUNTIME_MODULES`); the sources are embedded into the binary
(`RUNTIME_SOURCES`), so the shipped editor is one executable. A missing
or broken module is a fatal error — the editor reports it and exits
instead of starting without a working runtime. Extra modules are not
auto-loaded: `dofile` them from `init.lua`.

| File | Contents |
|---|---|
| `api.lua` | state, undo, kill ring, prefix args, command machinery, the `emacs` API |
| `motion.lua` | motion commands |
| `editing.lua` | editing commands, auto-indentation |
| `search.lua` | incremental search, its keymap and commands (`emacs.bind_isearch`) |
| `windows.lua` | window commands |
| `files.lua` | find/save/write/switch/kill-buffer, file completion |
| `modes.lua` | built-in major and minor modes (`minibuffer-mode` keymap included) |
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

-- isearch keymap: keys read while C-s / C-r is active (single keys).
-- Defaults: C-s/C-r repeat, C-y yank kill, C-w yank word, DEL delete
-- char, C-g abort, RET exit.
emacs.bind_isearch("C-p", "isearch-yank-kill")

-- synchronous reads (coroutine-based)
local name = emacs.read_string("Name: ", nil)
local yes  = emacs.read_yes_no("Sure? (y/n)")
local key  = emacs.read_key()            -- "C-x", "RET", "a", ...

-- major / minor modes
emacs.define_major_mode("txt-mode", {
  indent = 2,
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
  primitives
- `crates/lua/tests` unit tests drive the real Lua runtime against an
  in-memory editor: commands, undo, kill ring, prefix arguments, isearch
  (via coroutine resumes), indentation, dired, and the read-string /
  read-key / yes-no protocols
- `crates/app/tests` PTY integration tests: spawn the real `em` binary in
  a pseudo-terminal, send keystrokes, reconstruct the screen, and assert
  (editing, windows, search, modes, completion, dired, CLI)

CI (GitHub Actions) runs formatting, clippy, and the full test suite on
every push to `main` and every pull request.

## License

[Apache License 2.0](LICENSE)
