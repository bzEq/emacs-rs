-- File and buffer commands: find-file, save-buffer, write-file,
-- switch-to-buffer, kill-buffer, save-buffers-kill-terminal.

local function list_matching(list_dir, out_prefix, name_prefix)
  local matched = {}
  for _, e in ipairs(M.safe_read_dir(list_dir)) do
    local name = e.name
    if name:sub(1, #name_prefix) == name_prefix then
      local show = name_prefix:sub(1, 1) == "." or name:sub(1, 1) ~= "."
      if show then
        matched[#matched + 1] = { name = name, dir = e.is_dir }
      end
    end
  end
  table.sort(matched, function(a, b)
    if a.dir ~= b.dir then return a.dir end
    return string.lower(a.name) < string.lower(b.name)
  end)
  local out = {}
  for _, e in ipairs(matched) do
    out[#out + 1] = out_prefix .. e.name .. (e.dir and "/" or "")
  end
  return out
end

function M.complete_file_names(input)
  local expanded = raw.expand_tilde(input)
  local slash = expanded:match("^.*()/")
  if slash then
    return list_matching(expanded:sub(1, slash), expanded:sub(1, slash),
        expanded:sub(slash + 1))
  end
  local base = M.default_directory()
  return list_matching(base, "", expanded)
end

function M.mode_for_path(path)
  local lower = path:lower()
  if lower:match("%.rs$") then
    return "rust-mode"
  elseif lower:match("%.lua$") then
    return "lua-mode"
  end
  -- note: LuaJIT patterns do not support the `|` alternation operator
  local ext = lower:match("%.[%w]+$")
  if ext == ".cpp" or ext == ".cc" or ext == ".cxx" or ext == ".hpp" or ext == ".hh"
      or ext == ".h" or ext == ".inc" then
    return "cpp-mode"
  end
  return "fundamental-mode"
end

function M.open_file(path)
  local id = raw.find_buffer_by_path(path)
  if id then
    raw.select_buffer(id)
    return
  end
  local ok, newid = pcall(raw.open_file, path)
  if not ok then
    emacs.error("cannot open " .. path .. ": " .. tostring(newid))
    return
  end
  raw.set_buffer_mode(M.mode_for_path(path))
end

function M.save_buffer_at_id(id)
  if not raw.buffer_info(id).path then
    local name = emacs.read_string("File to save in: ", nil)
    if not name or name == "" then return false end
    raw.set_buffer_path(id, name)
    raw.set_buffer_name(id, name:match("([^/]+)$") or name)
  end
  M.run_hook("before_save")
  local ok, err = pcall(raw.save_buffer_to_disk, id)
  if not ok then
    emacs.error("Cannot save: " .. tostring(err))
    return false
  end
  raw.set_buffer_modified(id, false)
  M.run_hook("after_save")
  emacs.message("Wrote " .. (raw.buffer_info(id).path or ""))
  return true
end

local function confirm_save(fn)
  local name = raw.name()
  if raw.modified() then
    local yes = emacs.read_yes_no("Buffer " .. name .. " modified; save it? (y/n)")
    if yes == nil then return end
    if yes then
      if not M.save_buffer_at_id(raw.id()) then return end
    end
  end
  fn()
end

M.define("find-file", function()
  local base = M.default_directory()
  local prompt = "Find file: " .. base
  if base:sub(-1) ~= "/" then
    prompt = prompt .. "/"
  end
  local name = emacs.read_string(prompt, M.complete_file_names)
  if not name or name:match("^%s*$") == "" then return end
  local trimmed = name:match("^%s*(.-)%s*$")
  local expanded = raw.expand_tilde(trimmed)
  local path
  if expanded:match("^/") then
    path = expanded
  else
    path = base .. "/" .. expanded
  end
  confirm_save(function()
    if raw.is_dir(path) then
      dired_open(path)
      return
    end
    M.open_file(path)
  end)
end, "Open a file.")

M.define("save-buffer", function()
  M.save_buffer_at_id(raw.id())
end, "Save the current buffer to its file.")

M.define("write-file", function()
  local id = raw.id()
  local default = raw.buffer_info(id).path or ""
  local name = emacs.read_string("Write file: " .. default .. " ", nil)
  if not name or name == "" then return end
  raw.set_buffer_path(id, name)
  raw.set_buffer_name(id, name:match("([^/]+)$") or name)
  M.save_buffer_at_id(id)
end, "Save the current buffer to a new file.")

M.define("switch-to-buffer", function()
  local name = emacs.read_string("Switch to buffer: ", M.complete_buffer_names)
  if not name or name == "" then return end
  confirm_save(function()
    local found = nil
    for _, id in ipairs(raw.buffer_ids()) do
      if raw.buffer_info(id).name == name then found = id end
    end
    if found then
      raw.select_buffer(found)
    else
      raw.select_buffer(raw.new_buffer(name))
    end
  end)
end, "Switch to another buffer.")

M.define("kill-buffer", function()
  local default = raw.buffer_info(raw.id()).name
  local name = emacs.read_string("Kill buffer (default " .. default .. "): ",
      M.complete_buffer_names)
  if name == nil or name == "" then name = default end
  local target = nil
  for _, id in ipairs(raw.buffer_ids()) do
    if raw.buffer_info(id).name == name then target = id end
  end
  if not target then
    emacs.error("no buffer named " .. name)
    return
  end
  if raw.buffer_info(target).modified then
    local yes = emacs.read_yes_no("Buffer " .. name .. " modified; kill anyway? (y/n)")
    if yes then
      raw.kill_buffer(target)
    end
  else
    raw.kill_buffer(target)
  end
end, "Kill a buffer.")

M.define("save-buffers-kill-terminal", function()
  local modified = {}
  for _, id in ipairs(raw.buffer_ids()) do
    if raw.buffer_info(id).modified then
      modified[#modified + 1] = id
    end
  end
  local function confirm_all(idx)
    if idx > #modified then
      raw.set_quit(true)
      return
    end
    local id = modified[idx]
    local name = raw.buffer_info(id).name
    local yes = emacs.read_yes_no("Buffer " .. name .. " modified; save it? (y/n)")
    if yes == nil then return end
    if yes then
      M.save_buffer_at_id(id)
    end
    confirm_all(idx + 1)
  end
  confirm_all(1)
end, "Save buffers and exit.")
