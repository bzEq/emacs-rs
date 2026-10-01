-- clang-format.lua --- format code using clang-format, for emacs-rs.
--
-- A Lua port of the Emacs `clang-format' package: the buffer (or the
-- lines of the active region) is piped through the external
-- clang-format program, and the buffer is replaced with the result.
-- Only the requested lines are reformatted; clang-format's `--lines'
-- option leaves every other line byte-for-byte untouched.
--
-- Install by loading it from ~/.config/emacs-rs/init.lua:
--
--   dofile((os.getenv("XDG_CONFIG_HOME") or (os.getenv("HOME") .. "/.config"))
--          .. "/emacs-rs/clang-format.lua")
--
--   emacs.bind("C-c f", "clang-format-buffer")
--   emacs.bind("C-c r", "clang-format-region")
--
-- Commands:
--   M-x clang-format-buffer   format the whole buffer
--   M-x clang-format-region   format the active region, or the current
--                             line when no region is active
--
-- Configuration.  Set the fields on the global `clang_format' table
-- before loading the file, or later (they are read at command time):
--   clang_format.executable      clang-format binary (default: the
--                                `clang-format' found on PATH)
--   clang_format.style           value for --style; nil (default) lets
--                                clang-format read the .clang-format file
--                                in the buffer's directory or a parent
--   clang_format.fallback_style  style used when no .clang-format file is
--                                found.  "none" (the default) disables
--                                formatting in such buffers
--   clang_format.assume_filename file name passed to clang-format for
--                                style and language detection (default:
--                                the buffer's file)

clang_format = clang_format or {}

-- Quote a string for the shell (os.execute goes through /bin/sh).
local function shell_quote(s)
  return "'" .. s:gsub("'", "'\\''") .. "'"
end

local function read_file(path)
  local f = io.open(path, "rb")
  if not f then return nil end
  local text = f:read("*a")
  f:close()
  return text
end

local function write_file(path, text)
  local f = assert(io.open(path, "wb"))
  f:write(text)
  f:close()
end

-- Run clang-format with ARGS, feeding it INPUT on stdin.  Returns the
-- stdout text, or nil plus clang-format's stderr on failure.
local function run_clang_format(input, args)
  local in_path = os.tmpname()
  local out_path = os.tmpname()
  local err_path = os.tmpname()
  write_file(in_path, input)

  local executable = clang_format.executable or "clang-format"
  local cmd = shell_quote(executable) .. " " .. args
      .. " < " .. shell_quote(in_path)
      .. " > " .. shell_quote(out_path)
      .. " 2> " .. shell_quote(err_path)
  local status = os.execute(cmd)

  local output = read_file(out_path)
  local stderr = read_file(err_path) or ""
  os.remove(in_path)
  os.remove(out_path)
  os.remove(err_path)

  -- LuaJIT reports success as 0 (raw wait status) or true, depending on
  -- whether it was built with Lua 5.2 compatibility.
  if status ~= 0 and status ~= true then
    return nil, stderr
  end
  if output == nil then
    return nil, stderr
  end
  return output, stderr
end

-- Args for `clang-format': the buffer's file name for style discovery,
-- the configured style, and the inclusive 1-based line range to format.
local function build_args(path, start_line, end_line)
  local parts = {}
  local name = clang_format.assume_filename or path
  if name and name ~= "" then
    parts[#parts + 1] = "--assume-filename=" .. shell_quote(name)
  end
  if clang_format.style then
    parts[#parts + 1] = "--style=" .. shell_quote(clang_format.style)
  end
  parts[#parts + 1] = "--fallback-style="
      .. shell_quote(clang_format.fallback_style or "none")
  parts[#parts + 1] = string.format("--lines=%d:%d", start_line + 1, end_line + 1)
  return table.concat(parts, " ")
end

-- 0-based line containing char offset POS, without disturbing point.
local function char_to_line(pos)
  local saved = raw.point()
  raw.set_point(pos)
  local line = raw.line_of_point()
  raw.set_point(saved)
  return line
end

-- Restore point to a line/column saved before the buffer was replaced.
local function restore_position(line, col)
  local last = raw.len_lines() - 1
  if line > last then line = last end
  if col > raw.line_len(line) then col = raw.line_len(line) end
  raw.set_point(raw.line_start(line) + col)
end

local function first_line(text)
  return text:match("^[^\n]*") or ""
end

-- Format lines START_LINE..END_LINE (0-based, inclusive) of the buffer.
local function format_lines(start_line, end_line)
  if raw.read_only() then
    emacs.error("Buffer is read-only")
    return
  end

  local input = raw.buffer_string()
  if input == "" then
    emacs.message("(clang-format: empty buffer)")
    return
  end

  local last = raw.len_lines() - 1
  if start_line > last then start_line = last end
  if end_line > last then end_line = last end

  local output, stderr =
      run_clang_format(input, build_args(raw.path(), start_line, end_line))
  if not output then
    emacs.error("clang-format failed: " .. first_line(stderr))
    return
  end
  if output == "" then
    emacs.error("clang-format produced no output")
    return
  end

  if output ~= input then
    local line, col = raw.line_of_point(), raw.column()
    emacs.delete_range(0, raw.len_chars())
    emacs.insert(output)
    restore_position(line, col)
  end

  if stderr ~= "" then
    emacs.message("(clang-format: " .. first_line(stderr) .. ")")
  else
    emacs.message("(clang-format: success)")
  end
end

emacs.define_command("clang-format-buffer", function()
  format_lines(0, raw.len_lines() - 1)
end)

emacs.define_command("clang-format-region", function()
  local start_line, end_line
  local start_pos, end_pos = raw.region()
  if start_pos and end_pos then
    start_line = char_to_line(start_pos)
    end_line = char_to_line(end_pos)
    -- A region ending at the start of a line does not include that line.
    if end_line > start_line then
      local before = raw.get_text(end_pos - 1, end_pos)
      if before == "\n" or before == "\r" then
        end_line = end_line - 1
      end
    end
  else
    -- No active region: format the line at point (Emacs' package keeps
    -- the current statement; a single line is the closest match here).
    start_line = raw.line_of_point()
    end_line = start_line
  end
  format_lines(start_line, end_line)
end)
