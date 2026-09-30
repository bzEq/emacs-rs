-- Motion commands.

local function n_repeat(prefix, default)
  return math.max(prefix, default)
end

M.define("forward-char", function(prefix)
  for _ = 1, n_repeat(prefix, 1) do raw.move_char("forward") end
end, "Move point forward one character.")

M.define("backward-char", function(prefix)
  for _ = 1, n_repeat(prefix, 1) do raw.move_char("backward") end
end, "Move point backward one character.")

M.define("next-line", function(prefix)
  for _ = 1, n_repeat(prefix, 1) do raw.move_line("forward") end
end, "Move point down one line, preserving column.")

M.define("previous-line", function(prefix)
  for _ = 1, n_repeat(prefix, 1) do raw.move_line("backward") end
end, "Move point up one line, preserving column.")

M.define("move-beginning-of-line", function()
  raw.move_line_start()
end, "Move point to beginning of line.")

M.define("move-end-of-line", function()
  raw.move_line_end()
end, "Move point to end of line.")

M.define("forward-word", function(prefix)
  for _ = 1, n_repeat(prefix, 1) do raw.move_word("forward") end
end, "Move point forward one word.")

M.define("backward-word", function(prefix)
  for _ = 1, n_repeat(prefix, 1) do raw.move_word("backward") end
end, "Move point backward one word.")

M.define("beginning-of-buffer", function()
  raw.move_buffer_start()
end, "Move point to beginning of buffer.")

M.define("end-of-buffer", function()
  raw.move_buffer_end()
end, "Move point to end of buffer.")

M.define("scroll-up-command", function(prefix)
  for _ = 1, n_repeat(prefix, 1) do raw.page_down() end
end, "Scroll text upward one screenful.")

M.define("scroll-down-command", function(prefix)
  for _ = 1, n_repeat(prefix, 1) do raw.page_up() end
end, "Scroll text downward one screenful.")

M.define("recenter-top-bottom", function()
  raw.recenter()
end, "Center point in the window.")
