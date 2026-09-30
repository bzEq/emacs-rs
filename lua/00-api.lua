-- emacs-rs runtime core: state, undo, kill ring, prefix arguments, the
-- command machinery, and the user-facing `emacs` API. Every other defaults
-- module builds on this file.

M = {}

M.commands = {}
M.command_docs = {}
M.hooks = {}
M.mode_indent = {}   -- major mode name -> indentation unit in spaces
M.undo_lists = {}    -- buffer id -> undo entries
M.kill_ring = { entries = {}, current = 0 }
M.prefix = { digits = nil, negative = false, universal = 0 }
M.last_yank = nil
M.this_command = ""
M.last_command = ""
M.dired = {}         -- buffer id -> { dir = ..., entries = {...} }

-- ==== the emacs module: primitives plus Lua-side sugar =====================
--
-- `emacs` is a fresh table whose missing fields fall through to the `raw`
-- primitives; wrappers defined below override specific primitives (e.g. to
-- record undo) and must call `raw.*`, never `emacs.*`.

emacs = setmetatable({}, { __index = raw })

-- ---- editing wrappers that record undo ------------------------------------

local function undo_list(id)
  local l = M.undo_lists[id]
  if not l then
    l = {}
    M.undo_lists[id] = l
  end
  return l
end

local function undo_boundary(id)
  local l = undo_list(id)
  if l[#l] ~= "B" then
    l[#l + 1] = "B"
  end
end

local function record_undo(id, entry)
  undo_list(id)[#undo_list(id) + 1] = entry
end

function emacs.insert(text)
  if text == nil or text == "" then return end
  local pos = raw.point()
  raw.insert(text)
  record_undo(raw.id(), { t = "insert", pos = pos, len = raw.point() - pos })
end

function emacs.delete_range(a, b)
  local id = raw.id()
  local text = raw.delete_range(a, b)
  if text ~= "" then
    record_undo(id, { t = "delete", pos = a, text = text })
  end
  return text
end

function emacs.delete_backward()
  local p = raw.point()
  local start = raw.step_left(p)
  if start == p then return false end
  emacs.delete_range(start, p)
  return true
end

function emacs.delete_forward()
  local p = raw.point()
  local e = raw.step_right(p)
  if e == p then return false end
  emacs.delete_range(p, e)
  return true
end

function emacs.newline()
  emacs.insert("\n")
end

-- ---- undo -----------------------------------------------------------------

function M.undo()
  local id = raw.id()
  local l = undo_list(id)
  if #l == 0 then
    emacs.message("No further undo information")
    return
  end
  if l[#l] == "B" then
    table.remove(l)
  end
  while #l > 0 do
    local e = table.remove(l)
    if e == "B" then
      break
    end
    if e.t == "insert" then
      raw.delete_range(e.pos, e.pos + e.len)
      raw.set_point(e.pos)
    else -- delete: re-insert the text
      local newp = raw.insert_at(e.pos, e.text)
      raw.set_point(newp)
    end
  end
  undo_boundary(id)
end

-- ---- kill ring -------------------------------------------------------------

local function is_kill_command(name)
  return name == "kill-line" or name == "kill-region"
      or name == "kill-word" or name == "backward-kill-word"
end

function M.kill(text)
  if text == nil or text == "" then return end
  local append = is_kill_command(M.last_command)
  local kr = M.kill_ring
  if append and kr.entries[kr.current] then
    kr.entries[kr.current] = kr.entries[kr.current] .. text
  else
    kr.entries[#kr.entries + 1] = text
    if #kr.entries > 60 then
      table.remove(kr.entries, 1)
    end
    kr.current = #kr.entries
  end
end

function M.current_kill()
  return M.kill_ring.entries[M.kill_ring.current]
end

function M.kill_ring_pop()
  local kr = M.kill_ring
  if #kr.entries == 0 then return nil end
  kr.current = kr.current == 1 and #kr.entries or kr.current - 1
  return kr.entries[kr.current]
end

function emacs.kill(text)
  M.kill(text)
end

function emacs.yank()
  local t = M.current_kill()
  if not t then
    emacs.error("Kill ring is empty")
    return
  end
  emacs.insert(t)
end

-- ---- prefix arguments ------------------------------------------------------

function M.prefix_value()
  local p = M.prefix
  if p.digits then
    return p.negative and -p.digits or p.digits
  end
  if p.negative then return -1 end
  if p.universal > 0 then return 4 ^ p.universal end
  return 1
end

function M.prefix_active()
  local p = M.prefix
  return p.digits ~= nil or p.negative or p.universal > 0
end

local function is_prefix_setter(name)
  return name == "universal-argument" or name == "negative-argument"
      or name:match("^digit%-argument%-") ~= nil
end

-- ---- command machinery -----------------------------------------------------

function M.run_command(name, extra)
  local fn = M.commands[name]
  if not fn then
    emacs.error(name .. " is undefined")
    return
  end
  if M.last_command ~= name then
    undo_boundary(raw.id())
  end
  M.last_command = M.this_command
  M.this_command = name
  local prefix = M.prefix_value()
  local ok, err = pcall(fn, prefix, extra)
  M.last_command = name
  if not ok then
    err = tostring(err):gsub("^%[.-%]:%d+: ", "")
    emacs.error(err)
  end
  if not is_prefix_setter(name) then
    M.prefix = { digits = nil, negative = false, universal = 0 }
  end
end

function M.define(name, fn, doc)
  M.commands[name] = fn
  M.command_docs[name] = doc or ""
end

function emacs.define_command(name, fn)
  M.define(name, fn)
end

-- ---- synchronous reads (coroutine yields) ----------------------------------

function emacs.read_string(prompt, completion)
  local result = coroutine.yield({
    type = "read_string",
    prompt = prompt,
    completion = completion,
  })
  return result
end

function emacs.read_yes_no(prompt)
  return coroutine.yield({ type = "read_yes_no", prompt = prompt })
end

function emacs.read_key()
  return coroutine.yield({ type = "read_key" })
end

-- ---- hooks -----------------------------------------------------------------

function emacs.add_hook(name, fn)
  M.hooks[name] = M.hooks[name] or {}
  table.insert(M.hooks[name], fn)
end

function M.run_hook(name)
  local list = M.hooks[name]
  if not list then return end
  for _, fn in ipairs(list) do
    fn()
  end
end

-- ---- modes -----------------------------------------------------------------

function emacs.define_major_mode(name, opts)
  opts = opts or {}
  local lang = opts.language
  if lang ~= "rust" and lang ~= "lua" and lang ~= "cpp" then
    lang = nil
  end
  raw.register_mode_def(name, lang, opts.keymap)
  M.mode_indent[name] = opts.indent
  M.define(name, function()
    raw.set_buffer_mode(name)
  end)
end

function emacs.set_buffer_mode(name)
  raw.set_buffer_mode(name)
end

function emacs.define_minor_mode(name, opts)
  opts = opts or {}
  local lighter = opts.lighter or name
  raw.register_minor_def(name, lighter, opts.keymap)
  M.define(name .. "-mode", function()
    raw.minor_mode_toggle(name)
  end)
end

function emacs.minor_mode_enable(name)
  raw.minor_mode_enable(name)
end

function emacs.minor_mode_disable(name)
  raw.minor_mode_disable(name)
end

function emacs.minor_mode_toggle(name)
  raw.minor_mode_toggle(name)
end

function M.current_indent_unit()
  return M.mode_indent[raw.mode()]
end

-- ---- completion helpers ----------------------------------------------------

function M.complete_command_names(input)
  local names = {}
  for name in pairs(M.commands) do
    names[#names + 1] = name
  end
  local prefixed = {}
  for _, n in ipairs(names) do
    if n:sub(1, #input) == input then
      prefixed[#prefixed + 1] = n
    end
  end
  if #prefixed == 0 then
    for _, n in ipairs(names) do
      if n:find(input, 1, true) then
        prefixed[#prefixed + 1] = n
      end
    end
  end
  table.sort(prefixed)
  return prefixed
end

function M.complete_buffer_names(input)
  local names = {}
  for _, id in ipairs(raw.buffer_ids()) do
    local n = raw.buffer_info(id).name
    if n:sub(1, #input) == input then
      names[#names + 1] = n
    end
  end
  table.sort(names)
  return names
end

-- ---- filesystem helpers ----------------------------------------------------

function M.expand_tilde(input)
  return raw.expand_tilde(input)
end

local function safe_read_dir(path)
  local ok, entries = pcall(raw.read_dir, path)
  if not ok then return {} end
  return entries
end

M.safe_read_dir = safe_read_dir

function M.default_directory()
  local d = M.dired[raw.id()]
  if d then return d.dir end
  local p = raw.path()
  if p then
    local parent = p:match("^(.*)/[^/]+$")
    if parent then return parent end
    return raw.cwd()
  end
  return raw.cwd()
end

-- ---- startup ---------------------------------------------------------------

function M.startup(path)
  if not path or path == "" then return end
  if raw.is_dir(path) then
    dired_open(path)
  else
    M.open_file(path)
  end
end

_internals = {
  run_command = M.run_command,
  startup = M.startup,
}
