-- Dired: the directory editor, implemented in Lua on top of the raw
-- filesystem and buffer primitives. Each dired buffer's state (directory,
-- entries, marks) lives in M.dired keyed by buffer id.

local dired = {}

local function is_dot(name)
  return name == "." or name == ".."
end

local function rank(e)
  if e.name == "." then
    return 0
  elseif e.name == ".." then
    return 1
  elseif e.is_dir then
    return 2
  else
    return 3
  end
end

function dired.read_dir(dir)
  local entries = {
    { name = ".", path = dir .. "/.", is_dir = true, size = 0 },
    { name = "..", path = dir .. "/..", is_dir = true, size = 0 },
  }
  for _, e in ipairs(M.safe_read_dir(dir)) do
    entries[#entries + 1] = {
      name = e.name,
      path = dir .. "/" .. e.name,
      is_dir = e.is_dir,
      size = e.size,
    }
  end
  table.sort(entries, function(a, b)
    local ra, rb = rank(a), rank(b)
    if ra ~= rb then return ra < rb end
    return string.lower(a.name) < string.lower(b.name)
  end)
  return entries
end

function dired.open(dir, other_window)
  dir = raw.canonicalize(dir) or dir
  if not raw.is_dir(dir) then
    emacs.error(dir .. " is not a directory")
    return
  end
  local id = nil
  for _, bid in ipairs(raw.buffer_ids()) do
    local d = M.dired[bid]
    if d and d.dir == dir then id = bid end
  end
  if not id then
    id = raw.new_buffer(dir)
    M.dired[id] = { dir = dir, entries = {} }
    raw.set_buffer_read_only(id, true)
  end
  if other_window and raw.single_window() then
    raw.split_window_below()
  end
  raw.select_buffer(id)
  if raw.mode() ~= "dired-mode" then
    raw.set_buffer_mode("dired-mode")
  end
  dired.refresh()
end

function dired.refresh()
  -- the dired buffer may differ from the current buffer (a command's
  -- continuation after a prompt), so resolve it through the window
  local id = raw.selected_buffer_id()
  local d = M.dired[id]
  if not d then
    error("not a dired buffer")
  end
  local old_marks = {}
  for _, e in ipairs(d.entries) do
    if e.marked then old_marks[e.name] = true end
  end
  local entries = dired.read_dir(d.dir)
  for _, e in ipairs(entries) do
    e.marked = old_marks[e.name] or false
  end
  local text = "  " .. d.dir .. ":\n  total " .. #entries .. "\n"
  for _, e in ipairs(entries) do
    local size = e.is_dir and "" or tostring(e.size)
    local mark = e.marked and "*" or " "
    text = text .. mark .. string.format("%10s", size) .. "  " .. e.name
        .. (e.is_dir and "/" or "") .. "\n"
  end
  d.entries = entries
  raw.replace_buffer_content(id, text)
end

local function entry_at_point()
  local d = M.dired[raw.id()]
  if not d then return nil end
  local idx = raw.line_of_point() - 1
  if idx >= 1 and idx <= #d.entries then return idx end
  return nil
end

local function marked_or_current()
  local d = M.dired[raw.id()]
  if not d then return {} end
  local marked = {}
  for i, e in ipairs(d.entries) do
    if e.marked and not is_dot(e.name) then marked[#marked + 1] = i end
  end
  if #marked > 0 then return marked end
  local i = entry_at_point()
  if i and not is_dot(d.entries[i].name) then return { i } end
  return {}
end

function dired_open(path)
  dired.open(path, false)
end

M.define("dired", function()
  local default = M.default_directory()
  local input = emacs.read_string("Dired (directory): ", M.complete_file_names, default)
  local dir
  if not input or input:match("^%s*$") == "" then
    dir = default
  else
    local expanded = raw.expand_tilde(input:match("^%s*(.-)%s*$"))
    if expanded:match("^/") then
      dir = expanded
    else
      dir = default .. "/" .. expanded
    end
  end
  dired.open(dir, false)
end, "Open a directory in dired.")

M.define("dired-open", function()
  local i = entry_at_point()
  if not i then return end
  local e = M.dired[raw.id()].entries[i]
  if e.is_dir then
    dired.open(e.path, false)
  else
    M.open_file(e.path)
  end
end, "Open the entry under point.")

M.define("dired-open-other-window", function()
  local i = entry_at_point()
  if not i then return end
  local e = M.dired[raw.id()].entries[i]
  if e.is_dir then
    dired.open(e.path, true)
  else
    if raw.single_window() then raw.split_window_below() end
    M.open_file(e.path)
  end
end, "Open the entry under point in another window.")

M.define("dired-up-directory", function()
  local d = M.dired[raw.id()]
  local dir = d and d.dir or raw.cwd()
  local parent = dir:match("^(.*)/[^/]+$") or dir
  if parent == "" then parent = "/" end
  dired.open(parent, false)
end, "Go to the parent directory.")

M.define("dired-refresh", function()
  dired.refresh()
end, "Re-read the directory listing.")

M.define("dired-quit", function()
  raw.kill_buffer(raw.id())
end, "Kill the dired buffer.")

local function set_mark(marked)
  local i = entry_at_point()
  if not i then return end
  local d = M.dired[raw.id()]
  if is_dot(d.entries[i].name) then return end
  local line_start = raw.line_start(raw.line_of_point())
  raw.delete_range(line_start, line_start + 1)
  raw.set_point(line_start)
  raw.insert(marked and "*" or " ")
  raw.set_buffer_modified(raw.id(), false)
  d.entries[i].marked = marked
end

M.define("dired-mark", function()
  set_mark(true)
end, "Mark the entry under point.")

M.define("dired-unmark", function()
  set_mark(false)
end, "Unmark the entry under point.")

M.define("dired-unmark-all", function()
  local d = M.dired[raw.id()]
  if d then
    for _, e in ipairs(d.entries) do
      e.marked = false
    end
  end
  dired.refresh()
end, "Unmark all entries.")

M.define("dired-delete", function()
  local d = M.dired[raw.id()]
  local targets = {}
  for _, i in ipairs(marked_or_current()) do
    local e = d.entries[i]
    targets[#targets + 1] = { path = e.path, dir = e.is_dir }
  end
  if #targets == 0 then
    emacs.message("No files to delete")
    return
  end
  local names = {}
  for _, t in ipairs(targets) do
    names[#names + 1] = t.path:match("([^/]+)$") or ""
  end
  local yes = emacs.read_yes_no("Delete " .. table.concat(names, ", ") .. "? (y/n)")
  if yes == nil then return end
  if yes then
    for _, t in ipairs(targets) do
      local ok, err = pcall(raw.delete_file, t.path, t.dir)
      if not ok then
        emacs.error("cannot delete " .. t.path .. ": " .. tostring(err))
      end
    end
  end
  dired.refresh()
end, "Delete marked (or current) files.")

M.define("dired-rename", function()
  local i = entry_at_point()
  if not i then return end
  local d = M.dired[raw.id()]
  local src = d.entries[i].path
  local base = src:match("([^/]+)$") or ""
  if is_dot(base) then return end
  local input = emacs.read_string("Rename " .. base .. " to: ", nil)
  if not input or input:match("^%s*$") == "" then return end
  local parent = src:match("^(.*)/") or "/"
  local dest = parent .. "/" .. input:match("^%s*(.-)%s*$")
  local ok, err = pcall(raw.rename_file, src, dest)
  if not ok then
    emacs.error("cannot rename " .. src .. ": " .. tostring(err))
    return
  end
  dired.refresh()
end, "Rename the entry under point.")

M.define("dired-copy", function()
  local d = M.dired[raw.id()]
  local targets = {}
  for _, i in ipairs(marked_or_current()) do
    local e = d.entries[i]
    if not is_dot(e.name) then
      targets[#targets + 1] = e.path
    end
  end
  if #targets == 0 then
    emacs.message("No files to copy")
    return
  end
  local input = emacs.read_string("Copy to: ", nil)
  if not input or input:match("^%s*$") == "" then return end
  local dest_dir = input:match("^%s*(.-)%s*$")
  if not raw.is_dir(dest_dir) then
    emacs.error(dest_dir .. " is not a directory")
    return
  end
  for _, src in ipairs(targets) do
    local name = src:match("([^/]+)$")
    local ok, err = pcall(raw.copy_file, src, dest_dir .. "/" .. name)
    if not ok then
      emacs.error("cannot copy " .. src .. ": " .. tostring(err))
    end
  end
  dired.open(d.dir, false)
end, "Copy marked (or current) files.")

M.define("dired-create-directory", function()
  local d = M.dired[raw.id()]
  local input = emacs.read_string("Create directory: ", nil)
  if not input or input:match("^%s*$") == "" then return end
  local name = input:match("^%s*(.-)%s*$")
  local ok, err = pcall(raw.mkdir, d.dir .. "/" .. name)
  if not ok then
    emacs.error("cannot create " .. d.dir .. "/" .. name .. ": " .. tostring(err))
    return
  end
  dired.refresh()
end, "Create a directory.")

emacs.define_major_mode("dired-mode", {
  keymap = {
    RET = "dired-open",
    f = "dired-open",
    o = "dired-open-other-window",
    ["^"] = "dired-up-directory",
    g = "dired-refresh",
    q = "dired-quit",
    m = "dired-mark",
    u = "dired-unmark",
    U = "dired-unmark-all",
    D = "dired-delete",
    R = "dired-rename",
    C = "dired-copy",
    ["+"] = "dired-create-directory",
  },
})
