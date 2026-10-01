-- Motion commands.

-- Absolute repeat count and direction multiplier for a command, honoring
-- the prefix argument's sign: a negative prefix (M--) inverts the
-- direction of a directional command.
local function repeat_args(prefix, default)
  local n = prefix or default
  if n < 0 then return -n, -1 end
  return n, 1
end

-- The effective direction: `base` with the prefix's sign applied.
local function with_sign(base, sign)
  if sign < 0 then
    return base == "forward" and "backward" or "forward"
  end
  return base
end

M.define("forward-char", function(prefix)
  local n, sign = repeat_args(prefix, 1)
  for _ = 1, n do raw.move_char(with_sign("forward", sign)) end
end, "Move point forward one character.")

M.define("backward-char", function(prefix)
  local n, sign = repeat_args(prefix, 1)
  for _ = 1, n do raw.move_char(with_sign("backward", sign)) end
end, "Move point backward one character.")

M.define("next-line", function(prefix)
  local n, sign = repeat_args(prefix, 1)
  for _ = 1, n do raw.move_line(with_sign("forward", sign)) end
end, "Move point down one line, preserving column.")

M.define("previous-line", function(prefix)
  local n, sign = repeat_args(prefix, 1)
  for _ = 1, n do raw.move_line(with_sign("backward", sign)) end
end, "Move point up one line, preserving column.")

M.define("move-beginning-of-line", function()
  raw.move_line_start()
end, "Move point to beginning of line.")

M.define("move-end-of-line", function()
  raw.move_line_end()
end, "Move point to end of line.")

M.define("forward-word", function(prefix)
  local n, sign = repeat_args(prefix, 1)
  for _ = 1, n do raw.move_word(with_sign("forward", sign)) end
end, "Move point forward one word.")

M.define("backward-word", function(prefix)
  local n, sign = repeat_args(prefix, 1)
  for _ = 1, n do raw.move_word(with_sign("backward", sign)) end
end, "Move point backward one word.")

M.define("beginning-of-buffer", function()
  raw.move_buffer_start()
end, "Move point to beginning of buffer.")

M.define("end-of-buffer", function()
  raw.move_buffer_end()
end, "Move point to end of buffer.")

M.define("scroll-up-command", function(prefix)
  local n, sign = repeat_args(prefix, 1)
  for _ = 1, n do
    if with_sign("forward", sign) == "forward" then raw.page_down() else raw.page_up() end
  end
end, "Scroll text upward one screenful.")

M.define("scroll-down-command", function(prefix)
  local n, sign = repeat_args(prefix, 1)
  for _ = 1, n do
    if with_sign("backward", sign) == "backward" then raw.page_up() else raw.page_down() end
  end
end, "Scroll text downward one screenful.")

M.define("recenter-top-bottom", function()
  raw.recenter()
end, "Center point in the window.")
