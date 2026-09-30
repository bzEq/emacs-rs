-- Built-in major and minor modes. All behavior (indentation width,
-- language for highlighting) is configured here; users can redefine any of
-- these modes from their init.lua.

emacs.define_major_mode("fundamental-mode", {})

emacs.define_major_mode("rust-mode", {
  language = "rust",
  indent = 4,
})

emacs.define_major_mode("lua-mode", {
  language = "lua",
  indent = 4,
})

emacs.define_major_mode("cpp-mode", {
  language = "cpp",
  indent = 4,
})

emacs.define_minor_mode("line-numbers", {
  lighter = "Ln",
  doc = "Display line numbers in the gutter.",
})

M.define("line-numbers-mode", function()
  local on = raw.minor_mode_toggle("line-numbers")
  emacs.message(on and "Line numbers enabled" or "Line numbers disabled")
end, "Toggle line numbers in the gutter.")
