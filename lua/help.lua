-- M-x, describe-key, and describe-bindings.

M.define("execute-extended-command", function()
  local name = emacs.read_string("M-x ", M.complete_command_names)
  if not name or name == "" then return end
  M.run_command(name, nil)
end, "Run a command by name.")

M.define("describe-key", function()
  local keys = {}
  while true do
    local k = emacs.read_key()
    keys[#keys + 1] = k
    local seq = table.concat(keys, " ")
    local status, cmd = raw.lookup_key(seq)
    if status == "unbound" then
      emacs.error(seq .. " is undefined")
      return
    elseif status == "prefix" then
      emacs.message(seq .. "-")
    else
      local doc = M.command_docs[cmd] or ""
      local suffix = doc == "" and "" or (": " .. doc)
      emacs.message(seq .. " runs the command " .. cmd .. suffix)
      return
    end
  end
end, "Show what a key sequence runs.")

M.define("describe-bindings", function()
  local text = "Global key bindings:\n\n"
  for _, b in ipairs(raw.global_bindings()) do
    text = text .. b.seq .. "\t\t" .. b.cmd .. "\n"
  end
  local local_b = raw.local_bindings()
  if #local_b > 0 then
    text = text .. "\nLocal (mode) key bindings:\n\n"
    for _, b in ipairs(local_b) do
      text = text .. b.seq .. "\t\t" .. b.cmd .. "\n"
    end
  end
  for _, ms in ipairs(raw.minor_bindings()) do
    text = text .. "\nMinor mode " .. ms.name .. " key bindings:\n\n"
    for _, b in ipairs(ms.entries) do
      text = text .. b.seq .. "\t\t" .. b.cmd .. "\n"
    end
  end
  local help_id = nil
  for _, id in ipairs(raw.buffer_ids()) do
    if raw.buffer_info(id).name == "*Help*" then help_id = id end
  end
  if not help_id then
    help_id = raw.new_buffer("*Help*")
  end
  raw.select_buffer(help_id)
  raw.set_buffer_read_only(help_id, true)
  raw.replace_buffer_content(text)
end, "List all key bindings.")
