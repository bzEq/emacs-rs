-- Incremental search (isearch): C-s / C-r, implemented on top of
-- emacs.read_key() and the rope search primitives.

local isearch = {}

function isearch.run(forward)
  local query = ""
  local start = raw.point()
  local matched = nil
  local failed = false
  local wrapped = false

  local function prompt()
    local dir = forward and "" or " backward"
    local status = failed and "Failing " or ""
    local w = wrapped and " (wrapped)" or ""
    return status .. "I-search" .. dir .. ": " .. query .. w
  end

  local function step(restart)
    local len = raw.len_chars()
    local p = raw.point()
    local from
    if restart then
      from = p
    elseif forward then
      from = matched and (matched + 1) or (p + 1)
      if from > len then from = len end
    else
      from = matched or p
    end
    local found, found_end
    if forward then
      found, found_end = raw.search_forward(query, from)
      if not found and not restart and from > 0 then
        found, found_end = raw.search_forward(query, 0)
        wrapped = found ~= nil
      else
        wrapped = false
      end
    else
      found, found_end = raw.search_backward(query, from)
      if not found and not restart and from < len then
        found, found_end = raw.search_backward(query, len)
        wrapped = found ~= nil
      else
        wrapped = false
      end
    end
    if found then
      matched = found
      failed = false
      raw.set_point(found)
      raw.set_search_match(found, found_end)
    else
      matched = nil
      failed = query ~= ""
      wrapped = false
      raw.clear_search_match()
    end
  end

  local special = { RET = true, TAB = true, DEL = true, ESC = true, SPC = true }

  -- show the prompt immediately, before waiting for the first key
  -- (Emacs shows "I-search:" as soon as C-s is pressed)
  emacs.message(prompt())

  while true do
    local key = emacs.read_key()
    if key == "C-g" then
      raw.set_point(start)
      raw.deactivate_mark()
      raw.clear_search_match()
      emacs.message("Quit")
      return
    elseif key == "C-s" then
      if not forward then
        forward = true
        matched = nil
        step(true)
      else
        step(false)
      end
      emacs.message(prompt())
    elseif key == "C-r" then
      if forward then
        forward = false
        matched = nil
        step(true)
      else
        step(false)
      end
      emacs.message(prompt())
    elseif key == "C-w" then
      local p = raw.point()
      while true do
        local c = raw.char_at(p)
        if c and c:match("^[%w_]$") then
          query = query .. c
          p = p + 1
        else
          break
        end
      end
      step(true)
      emacs.message(prompt())
    elseif key == "DEL" then
      query = query:sub(1, -2)
      matched = nil
      if query == "" then
        raw.set_point(start)
      end
      step(true)
      emacs.message(prompt())
    elseif key == "RET" or key == "ESC" then
      raw.clear_search_match()
      return
    elseif key == "SPC" then
      query = query .. " "
      matched = nil
      step(true)
      emacs.message(prompt())
    elseif not special[key] and not key:find("-", 1, true) and not key:find("<", 1, true) then
      -- a printable character (possibly multibyte)
      query = query .. key
      matched = nil
      step(true)
      emacs.message(prompt())
    else
      raw.clear_search_match()
      emacs.replay_key()
      return
    end
  end
end

M.define("isearch-forward", function()
  isearch.run(true)
end, "Incremental search forward.")

M.define("isearch-backward", function()
  isearch.run(false)
end, "Incremental search backward.")
