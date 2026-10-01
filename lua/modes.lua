-- Built-in major and minor modes. All behavior (indentation width) is
-- configured here; users can redefine any of these modes from their
-- init.lua.

emacs.define_major_mode("fundamental-mode", {})

emacs.define_major_mode("rust-mode", {
  indent = 4,
})

emacs.define_major_mode("lua-mode", {
  indent = 4,
})

emacs.define_major_mode("cpp-mode", {
  indent = 4,
})

emacs.define_minor_mode("line-numbers", {
  lighter = "Ln",
  doc = "Display line numbers in the gutter.",
})

emacs.define_minor_mode("global-auto-revert", {
  lighter = "AR",
  doc = "Revert buffers when their files change on disk.",
})

M.define("line-numbers-mode", function()
  local on = raw.minor_mode_toggle("line-numbers")
  emacs.message(on and "Line numbers enabled" or "Line numbers disabled")
end, "Toggle line numbers in the gutter.")

M.define("global-auto-revert-mode", function()
  M.auto_revert_enabled = not M.auto_revert_enabled
  raw.minor_mode_toggle("global-auto-revert") -- cosmetic lighter
  emacs.message(M.auto_revert_enabled and "Auto-revert enabled"
      or "Auto-revert disabled")
end, "Toggle reverting buffers when their files change on disk.")
