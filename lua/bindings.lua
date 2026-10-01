-- The default global keymap.

local function bind(seq, cmd)
  raw.bind(seq, cmd)
end

-- motion
bind("C-f", "forward-char")
bind("<right>", "forward-char")
bind("C-b", "backward-char")
bind("<left>", "backward-char")
bind("C-n", "next-line")
bind("<down>", "next-line")
bind("C-p", "previous-line")
bind("<up>", "previous-line")
bind("C-a", "move-beginning-of-line")
bind("<home>", "move-beginning-of-line")
bind("C-e", "move-end-of-line")
bind("<end>", "move-end-of-line")
bind("M-f", "forward-word")
bind("M-b", "backward-word")
bind("M-<", "beginning-of-buffer")
bind("M->", "end-of-buffer")
bind("C-v", "scroll-up-command")
bind("<next>", "scroll-up-command")
bind("M-v", "scroll-down-command")
bind("<prior>", "scroll-down-command")
bind("C-l", "recenter-top-bottom")
-- editing
bind("RET", "newline-and-indent")
bind("C-j", "electric-newline-and-maybe-indent")
bind("TAB", "indent-for-tab-command")
bind("C-d", "delete-char")
bind("<delete>", "delete-char")
bind("DEL", "backward-delete-char")
bind("C-k", "kill-line")
bind("M-d", "kill-word")
bind("M-DEL", "backward-kill-word")
bind("C-w", "kill-region")
bind("M-w", "kill-ring-save")
bind("C-y", "yank")
bind("M-y", "yank-pop")
bind("C-/", "undo")
bind("C-_", "undo")
bind("C-SPC", "set-mark-command")
bind("C-@", "set-mark-command")
bind("C-g", "keyboard-quit")
bind("M-g M-g", "goto-line")
bind("C-u", "universal-argument")
bind("C--", "negative-argument")
bind("M--", "negative-argument")
-- files
bind("C-x C-f", "find-file")
bind("C-x C-s", "save-buffer")
bind("C-x C-w", "write-file")
bind("C-x b", "switch-to-buffer")
bind("C-x k", "kill-buffer")
bind("C-x u", "undo")
bind("C-x C-x", "exchange-point-and-mark")
bind("C-x C-c", "save-buffers-kill-terminal")
-- misc
bind("M-x", "execute-extended-command")
bind("C-h k", "describe-key")
bind("C-h b", "describe-bindings")
-- windows
bind("C-x 2", "split-window-below")
bind("C-x 3", "split-window-right")
bind("C-x 0", "delete-window")
bind("C-x 1", "delete-other-windows")
bind("C-x o", "other-window")
-- dired
bind("C-x d", "dired")
-- search
bind("C-s", "isearch-forward")
bind("C-r", "isearch-backward")
for d = 1, 9 do
  bind("C-" .. d, "digit-argument-" .. d)
  bind("M-" .. d, "digit-argument-" .. d)
end

-- The minibuffer keymap overrides the global map while the minibuffer is
-- active (Emacs's minibuffer-local-map). Only minibuffer-specific keys
-- live here; motion and editing keys fall through to the global map, so
-- the user's bindings operate on the minibuffer input, like Emacs.
local function mbind(seq, cmd)
  raw.bind_minibuffer(seq, cmd)
end
mbind("C-p", "minibuf-previous-history")
mbind("C-n", "minibuf-next-history")
mbind("<up>", "minibuf-previous-history")
mbind("<down>", "minibuf-next-history")
