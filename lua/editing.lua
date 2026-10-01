-- Editing commands: insertion, deletion, kill/yank, undo, marks, prefix
-- arguments, and auto-indentation.

-- ---- indentation -----------------------------------------------------------

local function first_non_ws_len(text)
  local ws = text:match("^[ \t]*") or ""
  return #ws
end

local function is_blank_line(idx)
  local t = raw.line_text(idx)
  return t:match("^[ \t]*$") ~= nil
end

local function prev_non_blank(idx)
  for l = idx - 1, 0, -1 do
    if not is_blank_line(l) then return l end
  end
  return nil
end

local function opens_indent(c, prev2)
  if c == "{" or c == "(" or c == "[" then return true end
  if c == "," or c == "=" then return true end
  if c == ":" then return prev2 == ":" end
  return false
end

local function closes_indent(word)
  return word == "end" or word == "else" or word == "elseif" or word == "until"
end

function M.compute_indent(idx, unit)
  local prev = prev_non_blank(idx)
  if not prev then return 0 end
  local text = raw.line_text(prev)
  local ws = text:match("^[ \t]*") or ""
  local indent = 0
  for i = 1, #ws do
    local ch = text:sub(i, i)
    indent = (ch == "\t") and (indent + unit - indent % unit) or (indent + 1)
  end
  local stripped = text:gsub("[ \t\r]*$", "")
  local nonws = stripped:gsub("[ \t]", "")
  local last1 = nonws:sub(-1, -1)
  local last2 = nonws:sub(-2, -2)
  if last2 == "" then last2 = nil end
  if last1 ~= "" and opens_indent(last1, last2) then
    indent = indent + unit
  end
  local prev_word = stripped:match("([%w_]+)$") or ""
  if closes_indent(prev_word) then
    indent = indent - unit
  end
  local this = raw.line_text(idx)
  local first = this:match("^[ \t]*([^ \t])")
  if first == "}" or first == ")" or first == "]" then
    indent = indent - unit
  end
  local word = this:match("^[ \t]*([%w_]+)") or ""
  if closes_indent(word) then
    indent = indent - unit
  end
  return math.max(indent, 0)
end

local function backward_delete_indent()
  local unit = M.current_indent_unit()
  if not unit then return false end
  local line = raw.line_of_point()
  local line_start = raw.line_start(line)
  local p = raw.point()
  local ws = raw.get_text(line_start, p)
  if ws == "" or ws:match("[^ \t]") then return false end
  local s = ws:find(" +$")
  local trailing = s and (#ws - s + 1) or 0
  local n = trailing > 0 and math.min(trailing, unit) or 1
  emacs.delete_range(p - n, p)
  return true
end

-- ---- commands --------------------------------------------------------------

M.define("self-insert-command", function(prefix, extra)
  if raw.read_only() then
    emacs.error("Buffer is read-only")
    return
  end
  local c = extra
  if c then
    for _ = 1, math.max(prefix, 1) do
      emacs.insert(c)
    end
  end
end, "Insert the typed character.")

M.define("newline", function()
  emacs.insert("\n")
end, "Insert a newline.")

M.define("newline-and-indent", function()
  local unit = M.current_indent_unit()
  emacs.insert("\n")
  if unit then
    emacs.insert(string.rep(" ", M.compute_indent(raw.line_of_point(), unit)))
  end
end, "Insert a newline and indent the new line.")

M.define("electric-newline-and-maybe-indent", function()
  local unit = M.current_indent_unit()
  local in_str = raw.in_comment_or_string()
  emacs.insert("\n")
  if unit and not in_str then
    emacs.insert(string.rep(" ", M.compute_indent(raw.line_of_point(), unit)))
  end
end, "Insert a newline and indent the new line, unless point is in a comment or string.")

M.define("indent-for-tab-command", function()
  local unit = M.current_indent_unit()
  if unit then
    local line = raw.line_of_point()
    local target = M.compute_indent(line, unit)
    local line_start = raw.line_start(line)
    local ws_len = first_non_ws_len(raw.line_text(line))
    emacs.delete_range(line_start, line_start + ws_len)
    raw.set_point(raw.line_start(line))
    emacs.insert(string.rep(" ", target))
  else
    emacs.insert("\t")
  end
end, "Indent the current line, or insert a tab.")

M.define("delete-char", function(prefix)
  local n = prefix or 1
  if n < 0 then
    for _ = 1, -n do emacs.delete_backward() end
  else
    for _ = 1, math.max(n, 1) do emacs.delete_forward() end
  end
end, "Delete the character at point.")

M.define("backward-delete-char", function(prefix)
  local n = prefix or 1
  if n < 0 then
    for _ = 1, -n do emacs.delete_forward() end
  else
    for _ = 1, math.max(n, 1) do
      if not backward_delete_indent() then
        emacs.delete_backward()
      end
    end
  end
end, "Delete the character before point.")

M.define("kill-line", function(prefix)
  local n = prefix or 1
  if n < 0 then
    -- kill the -n lines before point (Emacs semantics)
    local start_line = math.max(raw.line_of_point() + n, 0)
    local start = raw.line_start(start_line)
    M.kill(emacs.delete_range(start, raw.point()))
    return
  end
  local killed = ""
  for _ = 1, math.max(n, 1) do
    local line = raw.line_of_point()
    local line_start = raw.line_start(line)
    local eol = line_start + raw.line_len(line)
    if raw.point() == eol then
      killed = killed .. emacs.delete_range(eol, raw.step_right(eol))
    else
      killed = killed .. emacs.delete_range(raw.point(), eol)
    end
  end
  M.kill(killed)
end, "Kill the rest of the current line.")

M.define("kill-word", function(prefix)
  local n = prefix or 1
  if n < 0 then
    local killed = ""
    for _ = 1, -n do
      local e = raw.point()
      raw.move_word("backward")
      killed = emacs.delete_range(raw.point(), e) .. killed
    end
    M.kill(killed)
    return
  end
  local killed = ""
  for _ = 1, math.max(n, 1) do
    local start = raw.point()
    raw.move_word("forward")
    killed = killed .. emacs.delete_range(start, raw.point())
  end
  M.kill(killed)
end, "Kill the word after point.")

M.define("backward-kill-word", function(prefix)
  local n = prefix or 1
  if n < 0 then
    local killed = ""
    for _ = 1, -n do
      local start = raw.point()
      raw.move_word("forward")
      killed = killed .. emacs.delete_range(start, raw.point())
    end
    M.kill(killed)
    return
  end
  local killed = ""
  for _ = 1, math.max(n, 1) do
    local e = raw.point()
    raw.move_word("backward")
    killed = killed .. emacs.delete_range(raw.point(), e)
  end
  M.kill(killed)
end, "Kill the word before point.")

M.define("kill-region", function()
  local s, e = raw.region()
  if not s then
    error("The mark is not set now, so there is no region")
  end
  M.kill(emacs.delete_range(s, e))
end, "Kill the text between point and mark.")

M.define("kill-ring-save", function()
  local s, e = raw.region()
  if not s then
    error("The mark is not set now, so there is no region")
  end
  M.kill(raw.get_text(s, e))
end, "Copy the region to the kill ring.")

M.define("yank", function(prefix)
  if not M.yank_current(prefix) then
    error("Kill ring is empty")
  end
end, "Insert the most recent kill.")

M.define("yank-pop", function()
  if M.last_command ~= "yank" and M.last_command ~= "yank-pop" then
    error("Previous command was not a yank")
  end
  if #M.kill_ring.entries < 2 then
    error("Only one element in the kill ring")
  end
  if not M.last_yank then
    error("Previous command was not a yank")
  end
  -- the buffer must still contain exactly the yanked text
  local cur = raw.get_text(M.last_yank.pos, M.last_yank.pos + M.last_yank.len)
  if cur ~= M.last_yank.text then
    error("The yanked text has changed; yank-pop is not possible")
  end
  local t = M.kill_ring_pop()
  if not t then
    error("Kill ring is empty")
  end
  emacs.delete_range(M.last_yank.pos, M.last_yank.pos + M.last_yank.len)
  raw.set_point(M.last_yank.pos)
  emacs.insert(t)
  M.last_yank = {
    id = M.last_yank.id,
    pos = M.last_yank.pos,
    len = raw.point() - M.last_yank.pos,
    text = raw.get_text(M.last_yank.pos, raw.point()),
  }
end, "Replace yanked text with a previous kill.")

M.define("undo", function()
  M.undo()
end, "Undo the last change.")

M.define("set-mark-command", function()
  raw.set_mark(raw.point())
  emacs.message("Mark set")
end, "Set the mark where point is.")

local function goto_line_number(n)
  n = math.floor(n)
  if n < 1 or n ~= n then
    emacs.error("Goto line requires a positive line number")
    return
  end
  local last = raw.len_lines() - 1
  local line = math.min(n - 1, last)
  local old = raw.point()
  raw.set_mark(old)
  raw.set_point(raw.line_start(line))
  emacs.message("Goto line " .. (raw.line_of_point() + 1))
end

M.define("goto-line", function(prefix)
  if M.prefix_active() then
    goto_line_number(prefix)
    return
  end
  local max = raw.len_lines()
  local input = emacs.read_string("Goto line (1-" .. max .. "): ", nil)
  if not input or input:match("^%s*$") then return end
  local n = tonumber(input:match("^%s*(.-)%s*$"))
  if not n then
    emacs.error("not a number")
    return
  end
  goto_line_number(n)
end, "Move point to the beginning of a line, setting the mark.")

M.define("exchange-point-and-mark", function()
  raw.exchange_point_and_mark()
end, "Swap point and mark.")

M.define("keyboard-quit", function()
  M.prefix = { digits = nil, negative = false, universal = 0 }
  emacs.message("Quit")
end, "Abort the current operation.")

M.define("universal-argument", function()
  M.prefix.universal = M.prefix.universal + 1
end, "Begin a numeric argument (4x).")

M.define("negative-argument", function()
  M.prefix.negative = not M.prefix.negative
end, "Begin a negative numeric argument.")

for d = 1, 9 do
  M.define("digit-argument-" .. d, function()
    M.prefix.digits = (M.prefix.digits or 0) * 10 + d
  end, "Start a numeric argument digit.")
end
