-- Incremental search (isearch): C-s / C-r.  The key loop reads keys with
-- emacs.read_key() and dispatches them through the `isearch' keymap
-- (M.isearch_bindings); the commands operate on the shared state in
-- M.isearch, so they are ordinary commands (M-x-able) and the keymap can
-- be customized from init.lua:
--
--   emacs.bind_isearch("C-p", "isearch-yank-kill")
--
-- Unbound keys that are printable append to the search string
-- (Emacs `isearch-printing-char'); any other unbound key ends the search
-- and is replayed against the buffer.

-- Emacs `isearch-text-char-description': show control characters (from a
-- yanked kill, say) in the echo area instead of breaking the display.
local function describe(text)
  text = text:gsub("\r", "^M")
  text = text:gsub("\n", "^J")
  text = text:gsub("\t", "^I")
  return text
end

-- Remove the last character of TEXT (DEL); a multibyte character is one
-- character even though it spans several bytes.
local function chop_last_char(text)
  local i = #text
  while i > 0 and text:byte(i) >= 0x80 and text:byte(i) < 0xC0 do
    i = i - 1
  end
  return text:sub(1, i - 1)
end

-- Shared isearch state, read and written by the commands below.
M.isearch = {
  active = false,
  query = "",
  start = 0,
  matched = nil,
  failed = false,
  wrapped = false,
  forward = true,
}

local isearch = {}

function isearch.prompt()
  local s = M.isearch
  local dir = s.forward and "" or " backward"
  local status = s.failed and "Failing " or ""
  local w = s.wrapped and " (wrapped)" or ""
  return status .. "I-search" .. dir .. ": " .. describe(s.query) .. w
end

-- Extend the search string by TEXT and search it from the current match.
local function push_query(text)
  local s = M.isearch
  s.query = s.query .. text
  s.matched = nil
  isearch.step(true)
  emacs.message(isearch.prompt())
end

-- Search for the current query.  With RESTART, start over from point;
-- otherwise continue from the previous match (wrapping at the ends).
function isearch.step(restart)
  local s = M.isearch
  local len = raw.len_chars()
  local p = raw.point()
  local from
  if restart then
    from = p
  elseif s.forward then
    from = s.matched and (s.matched + 1) or (p + 1)
    if from > len then from = len end
  else
    from = s.matched or p
  end
  local found, found_end
  if s.forward then
    found, found_end = raw.search_forward(s.query, from)
    if not found and not restart and from > 0 then
      found, found_end = raw.search_forward(s.query, 0)
      s.wrapped = found ~= nil
    else
      s.wrapped = false
    end
  else
    found, found_end = raw.search_backward(s.query, from)
    if not found and not restart and from < len then
      found, found_end = raw.search_backward(s.query, len)
      s.wrapped = found ~= nil
    else
      s.wrapped = false
    end
  end
  if found then
    s.matched = found
    s.failed = false
    raw.set_point(found)
    raw.set_search_match(found, found_end)
  else
    s.matched = nil
    s.failed = s.query ~= ""
    s.wrapped = false
    raw.clear_search_match()
  end
end

-- The isearch keymap: key -> command name (single keys).  Rebind with
-- emacs.bind_isearch("C-p", "isearch-yank-kill").
M.isearch_bindings = {
  ["C-s"] = "isearch-repeat-forward",
  ["C-r"] = "isearch-repeat-backward",
  ["C-g"] = "isearch-abort",
  ["C-y"] = "isearch-yank-kill",
  ["C-w"] = "isearch-yank-word",
  ["DEL"] = "isearch-delete-char",
  ["RET"] = "isearch-exit",
  ["ESC"] = "isearch-exit",
  ["SPC"] = "isearch-printing-char",
}

function emacs.bind_isearch(seq, cmd)
  M.isearch_bindings[seq] = cmd
end

local special = { RET = true, TAB = true, DEL = true, ESC = true, SPC = true }

-- A key is printable text unless it is a named key, a modifier combo
-- ("C-x", "M--", ...), or an angle-bracket key name ("<left>", ...).
-- A bare "-" or "<" is an ordinary character (LuaJIT patterns have no
-- alternation, hence the explicit checks).
local function is_printable(key)
  if special[key] then return false end
  if key ~= "-" and key:find("-", 1, true) then return false end
  if key:sub(1, 1) == "<" and key:sub(-1) == ">" then return false end
  return true
end

-- ---- isearch commands -------------------------------------------------------

M.define("isearch-printing-char", function(prefix, key)
  local s = M.isearch
  if not s.active or not key then return end
  if key == "SPC" then key = " " end
  push_query(key)
end, "Append the typed character to the search string.")

M.define("isearch-repeat-forward", function()
  local s = M.isearch
  if not s.active then return end
  if not s.forward then
    s.forward = true
    s.matched = nil
    isearch.step(true)
  else
    isearch.step(false)
  end
  emacs.message(isearch.prompt())
end, "Repeat the search forward.")

M.define("isearch-repeat-backward", function()
  local s = M.isearch
  if not s.active then return end
  if s.forward then
    s.forward = false
    s.matched = nil
    isearch.step(true)
  else
    isearch.step(false)
  end
  emacs.message(isearch.prompt())
end, "Repeat the search backward.")

M.define("isearch-yank-kill", function()
  local s = M.isearch
  if not s.active then return end
  local text = M.current_kill()
  if text then
    push_query(text)
  else
    emacs.message(isearch.prompt())
  end
end, "Yank the latest kill into the search string.")

M.define("isearch-yank-word", function()
  local s = M.isearch
  if not s.active then return end
  local p = raw.point()
  while true do
    local c = raw.char_at(p)
    if c and c:match("^[%w_]$") then
      s.query = s.query .. c
      p = p + 1
    else
      break
    end
  end
  s.matched = nil
  isearch.step(true)
  emacs.message(isearch.prompt())
end, "Append the word at point to the search string.")

M.define("isearch-delete-char", function()
  local s = M.isearch
  if not s.active then return end
  s.query = chop_last_char(s.query)
  s.matched = nil
  if s.query == "" then
    raw.set_point(s.start)
  end
  isearch.step(true)
  emacs.message(isearch.prompt())
end, "Delete the last character of the search string.")

M.define("isearch-abort", function()
  local s = M.isearch
  if not s.active then return end
  raw.set_point(s.start)
  raw.deactivate_mark()
  raw.clear_search_match()
  s.active = false
  emacs.message("Quit")
end, "Abort the search, restoring point to where it started.")

M.define("isearch-exit", function()
  local s = M.isearch
  s.active = false
  raw.clear_search_match()
end, "End the search at the current match.")

-- ---- entry points ------------------------------------------------------------

local function run(forward)
  local s = M.isearch
  s.active = true
  s.query = ""
  s.start = raw.point()
  s.matched = nil
  s.failed = false
  s.wrapped = false
  s.forward = forward
  emacs.message(isearch.prompt())

  while s.active do
    local key = emacs.read_key()
    local cmd = M.isearch_bindings[key]
    if cmd then
      M.run_command(cmd, key)
    elseif is_printable(key) then
      M.run_command("isearch-printing-char", key)
    else
      -- Keep honoring a `yank' rebinding from the normal keymaps, so a
      -- custom yank key works in the search too; anything else ends the
      -- search and runs normally.
      local status, global_cmd = raw.lookup_key(key)
      if status == "command" and global_cmd == "yank" then
        M.run_command("isearch-yank-kill")
      else
        raw.clear_search_match()
        emacs.replay_key()
        s.active = false
      end
    end
  end
end

M.define("isearch-forward", function()
  run(true)
end, "Incremental search forward.")

M.define("isearch-backward", function()
  run(false)
end, "Incremental search backward.")
