-- Window commands.

M.define("split-window-below", function()
  raw.split_window_below()
end, "Split the selected window in two, one above the other.")

M.define("split-window-right", function()
  raw.split_window_right()
end, "Split the selected window in two, side by side.")

M.define("delete-window", function()
  if not raw.delete_window() then
    emacs.error("Attempt to delete the sole window")
  end
end, "Delete the selected window.")

M.define("delete-other-windows", function()
  raw.delete_other_windows()
end, "Make the selected window fill its frame.")

M.define("other-window", function()
  raw.other_window()
end, "Select the next window.")
