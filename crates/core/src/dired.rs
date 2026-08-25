//! Dired: the directory editor. Lists a directory in a buffer with a local
//! `dired-mode` keymap for navigation, marking, and file operations.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::buffer::Buffer;
use crate::editor::Editor;
use crate::keymap::Keymap;
use crate::mode::ModeDef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiredEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub marked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiredState {
    pub dir: PathBuf,
    pub entries: Vec<DiredEntry>,
}

impl DiredState {
    /// Number of non-entry lines at the top of the buffer (directory line,
    /// total line).
    pub fn header_lines(&self) -> usize {
        2
    }
}

fn rank(e: &DiredEntry) -> u8 {
    if e.name == "." {
        0
    } else if e.name == ".." {
        1
    } else if e.is_dir {
        2
    } else {
        3
    }
}

fn is_dot_entry(name: &str) -> bool {
    name == "." || name == ".."
}

/// Read and sort a directory listing (with explicit `.` and `..` entries).
pub fn read_dir(dir: &Path) -> Result<Vec<DiredEntry>> {
    let mut entries = Vec::new();
    entries.push(DiredEntry {
        name: ".".into(),
        path: dir.join("."),
        is_dir: true,
        size: 0,
        marked: false,
    });
    entries.push(DiredEntry {
        name: "..".into(),
        path: dir.join(".."),
        is_dir: true,
        size: 0,
        marked: false,
    });
    let rd = fs::read_dir(dir).map_err(|e| anyhow!("cannot read {}: {e}", dir.display()))?;
    for entry in rd {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(md) = entry.metadata() else { continue };
        entries.push(DiredEntry {
            name,
            path: entry.path(),
            is_dir: md.is_dir(),
            size: md.len(),
            marked: false,
        });
    }
    entries.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// The entry index under point, if point is on an entry line.
pub fn entry_at_point(ed: &Editor) -> Option<usize> {
    let buf = ed.buf();
    let d = buf.dired()?;
    let line = buf.line_of_point();
    let idx = line.checked_sub(d.header_lines())?;
    (idx < d.entries.len()).then_some(idx)
}

/// Marked entries, or the entry under point if nothing is marked.
fn marked_or_current(ed: &Editor) -> Vec<usize> {
    let buf = ed.buf();
    let Some(d) = buf.dired() else {
        return Vec::new();
    };
    let marked: Vec<usize> = d
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.marked && !is_dot_entry(&e.name))
        .map(|(i, _)| i)
        .collect();
    if !marked.is_empty() {
        return marked;
    }
    entry_at_point(ed)
        .into_iter()
        .filter(|&i| !is_dot_entry(&d.entries[i].name))
        .collect()
}

/// The directory that file operations should start from: the current
/// dired buffer's directory, else the current file's parent directory,
/// else the process working directory.
pub fn default_dir(ed: &Editor) -> PathBuf {
    if let Some(d) = ed.buf().dired() {
        return d.dir.clone();
    }
    if let Some(p) = ed.buf().path() {
        if let Some(parent) = p.parent() {
            return parent.to_path_buf();
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

/// Expand a leading `~/` in a path.
pub fn expand_tilde(input: &str) -> String {
    if let Some(rest) = input.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            let home = Path::new(&home);
            return format!("{}/{}", home.display(), rest);
        }
    }
    input.to_string()
}

/// List `list_dir` and return entries matching `name_prefix`, prefixed
/// with `out_prefix` (empty for relative completions). Directories sort
/// first with a trailing `/`; dotfiles are hidden unless the prefix
/// starts with `.`.
fn list_matching(list_dir: &str, out_prefix: &str, name_prefix: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(list_dir) else {
        return Vec::new();
    };
    let mut entries: Vec<(String, bool)> = rd
        .filter_map(|e| e.ok())
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            (name, is_dir)
        })
        .filter(|(name, _)| {
            name.starts_with(name_prefix)
                && (name_prefix.starts_with('.') || !name.starts_with('.'))
        })
        .collect();
    entries.sort_by(|a, b| {
        b.1.cmp(&a.1) // directories first
            .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
    });
    entries
        .into_iter()
        .map(|(name, is_dir)| {
            let mut full = out_prefix.to_string();
            full.push_str(&name);
            if is_dir {
                full.push('/');
            }
            full
        })
        .collect()
}

/// Completion over file names: for inputs containing a `/`, the directory
/// part of the input is listed; otherwise the editor's default directory
/// (dired dir > current file's parent > cwd) is used and candidates are
/// returned as relative names.
pub fn complete_file_names(ed: &Editor, input: &str) -> Vec<String> {
    let expanded = expand_tilde(input);
    match expanded.rfind('/') {
        Some(i) => list_matching(&expanded[..i + 1], &expanded[..i + 1], &expanded[i + 1..]),
        None => {
            let base = default_dir(ed);
            list_matching(&base.to_string_lossy(), "", &expanded)
        }
    }
}

/// Open a directory in a dired buffer (creating it if needed), optionally
/// in another window.
pub fn open_dir(ed: &mut Editor, dir: &Path, other_window: bool) -> Result<()> {
    let dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    if !dir.is_dir() {
        ed.error(format!("{} is not a directory", dir.display()));
        return Ok(());
    }
    let id = match ed
        .buffers()
        .iter()
        .position(|b| b.dired().is_some_and(|d| d.dir == dir))
    {
        Some(idx) => ed.buffers()[idx].id,
        None => {
            let mut buf = Buffer::new(dir.display().to_string());
            buf.set_dired(Some(DiredState {
                dir: dir.clone(),
                entries: Vec::new(),
            }));
            buf.set_read_only(true);
            let id = buf.id;
            ed.add_buffer(buf);
            id
        }
    };
    if other_window && ed.single_window() {
        ed.split_window(crate::window::Split::Vertical);
    }
    let idx = ed.buffer_index(id);
    if ed.buffers()[idx].mode().name != "dired-mode" {
        ed.set_buffer_mode_by_name(idx, "dired-mode")?;
    }
    ed.set_selected_buffer(id);
    refresh(ed)
}

/// Re-read the directory and rebuild the buffer text, preserving marks.
pub fn refresh(ed: &mut Editor) -> Result<()> {
    let dir = ed
        .buf()
        .dired()
        .map(|d| d.dir.clone())
        .ok_or_else(|| anyhow!("not a dired buffer"))?;
    let old_marks: Vec<String> = ed
        .buf()
        .dired()
        .map(|d| {
            d.entries
                .iter()
                .filter(|e| e.marked)
                .map(|e| e.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let mut entries = read_dir(&dir)?;
    for e in &mut entries {
        e.marked = old_marks.contains(&e.name);
    }
    let mut text = format!("  {}:\n  total {}\n", dir.display(), entries.len());
    for e in &entries {
        let size = if e.is_dir {
            String::new()
        } else {
            e.size.to_string()
        };
        let mark = if e.marked { '*' } else { ' ' };
        let slash = if e.is_dir { "/" } else { "" };
        text.push_str(&format!("{mark}{size:>10}  {}{}\n", e.name, slash));
    }
    let idx = ed.selected_buffer_index();
    ed.buffers_mut()[idx].set_dired(Some(DiredState { dir, entries }));
    let buf = ed.buf_mut();
    buf.undo_boundary();
    let len = buf.rope().len_chars();
    if len > 0 {
        let _ = buf.delete_range(0, len);
    }
    buf.move_to_buffer_start();
    buf.insert(&text);
    buf.move_to_buffer_start();
    buf.set_modified(false);
    Ok(())
}

fn open_file(ed: &mut Editor, path: &Path) -> Result<()> {
    if let Some(idx) = ed.find_buffer_by_path(path) {
        let id = ed.buffers()[idx].id;
        ed.set_selected_buffer(id);
        return Ok(());
    }
    match Buffer::load_file(path) {
        Ok(buf) => {
            let id = buf.id;
            ed.add_buffer(buf);
            ed.set_selected_buffer(id);
        }
        Err(e) => ed.error(format!("cannot open {}: {e}", path.display())),
    }
    Ok(())
}

// --- commands ---------------------------------------------------------------

fn cmd_dired(ed: &mut Editor) -> Result<()> {
    let default = default_dir(ed);
    ed.read_string(
        format!("Dired (directory): {} ", default.display()),
        Some(complete_file_names),
        Box::new(move |ed, input| {
            let dir = if input.trim().is_empty() {
                default
            } else {
                let expanded = expand_tilde(input.trim());
                let p = PathBuf::from(&expanded);
                if p.is_absolute() {
                    p
                } else {
                    default.join(p)
                }
            };
            open_dir(ed, &dir, false)
        }),
    );
    Ok(())
}

fn cmd_dired_open(ed: &mut Editor) -> Result<()> {
    let Some(idx) = entry_at_point(ed) else {
        return Ok(());
    };
    let (path, is_dir) = {
        let d = ed.buf().dired().expect("dired buffer");
        (d.entries[idx].path.clone(), d.entries[idx].is_dir)
    };
    if is_dir {
        open_dir(ed, &path, false)
    } else {
        open_file(ed, &path)
    }
}

fn cmd_dired_open_other_window(ed: &mut Editor) -> Result<()> {
    let Some(idx) = entry_at_point(ed) else {
        return Ok(());
    };
    let (path, is_dir) = {
        let d = ed.buf().dired().expect("dired buffer");
        (d.entries[idx].path.clone(), d.entries[idx].is_dir)
    };
    if is_dir {
        open_dir(ed, &path, true)
    } else {
        if ed.single_window() {
            ed.split_window(crate::window::Split::Vertical);
        }
        open_file(ed, &path)
    }
}

fn cmd_dired_up_directory(ed: &mut Editor) -> Result<()> {
    let dir = ed.buf().dired().map(|d| d.dir.clone()).unwrap_or_default();
    let parent = dir.parent().unwrap_or(&dir).to_path_buf();
    open_dir(ed, &parent, false)
}

fn cmd_dired_refresh(ed: &mut Editor) -> Result<()> {
    refresh(ed)
}

fn cmd_dired_quit(ed: &mut Editor) -> Result<()> {
    let idx = ed.selected_buffer_index();
    ed.kill_buffer_at(idx);
    Ok(())
}

fn set_mark(ed: &mut Editor, marked: bool) -> Result<()> {
    let Some(idx) = entry_at_point(ed) else {
        return Ok(());
    };
    if is_dot_entry(&ed.buf().dired().expect("dired buffer").entries[idx].name) {
        return Ok(());
    }
    let line = ed.buf().line_of_point();
    let line_start = ed.buf().rope().line_to_char(line);
    let buf = ed.buf_mut();
    let _ = buf.delete_range(line_start, line_start + 1);
    buf.insert(if marked { "*" } else { " " });
    buf.set_modified(false);
    if let Some(d) = buf.dired_mut() {
        d.entries[idx].marked = marked;
    }
    Ok(())
}

fn cmd_dired_mark(ed: &mut Editor) -> Result<()> {
    set_mark(ed, true)
}

fn cmd_dired_unmark(ed: &mut Editor) -> Result<()> {
    set_mark(ed, false)
}

fn cmd_dired_unmark_all(ed: &mut Editor) -> Result<()> {
    let idx = ed.selected_buffer_index();
    if let Some(d) = ed.buffers_mut()[idx].dired_mut() {
        for e in &mut d.entries {
            e.marked = false;
        }
    }
    refresh(ed)
}

fn cmd_dired_delete(ed: &mut Editor) -> Result<()> {
    let targets: Vec<(PathBuf, bool)> = marked_or_current(ed)
        .into_iter()
        .map(|i| {
            let d = ed.buf().dired().expect("dired buffer");
            let e = &d.entries[i];
            (e.path.clone(), e.is_dir)
        })
        .collect();
    if targets.is_empty() {
        ed.message("No files to delete");
        return Ok(());
    }
    let names: Vec<String> = targets
        .iter()
        .map(|(p, _)| {
            p.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();
    ed.read_yes_no(
        format!("Delete {}? (y/n)", names.join(", ")),
        Box::new(move |ed, yes| {
            if yes {
                for (path, is_dir) in &targets {
                    let r = if *is_dir {
                        fs::remove_dir_all(path)
                    } else {
                        fs::remove_file(path)
                    };
                    if let Err(e) = r {
                        ed.error(format!("cannot delete {}: {e}", path.display()));
                    }
                }
            }
            refresh(ed)
        }),
    );
    Ok(())
}

fn cmd_dired_rename(ed: &mut Editor) -> Result<()> {
    let Some(idx) = entry_at_point(ed) else {
        return Ok(());
    };
    let src = ed.buf().dired().expect("dired buffer").entries[idx]
        .path
        .clone();
    if is_dot_entry(
        &src.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
    ) {
        return Ok(());
    }
    let default = src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    ed.read_string(
        format!("Rename {default} to: "),
        None,
        Box::new(move |ed, input| {
            if input.trim().is_empty() {
                return Ok(());
            }
            let dest = src.parent().unwrap_or(Path::new("/")).join(input.trim());
            if let Err(e) = fs::rename(&src, &dest) {
                ed.error(format!("cannot rename {}: {e}", src.display()));
                return Ok(());
            }
            refresh(ed)
        }),
    );
    Ok(())
}

fn copy_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    let md = fs::metadata(src)?;
    if md.is_dir() {
        fs::create_dir_all(dest)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dest.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(src, dest).map(|_| ())
    }
}

fn cmd_dired_copy(ed: &mut Editor) -> Result<()> {
    let targets: Vec<PathBuf> = marked_or_current(ed)
        .into_iter()
        .filter_map(|i| {
            let d = ed.buf().dired().expect("dired buffer");
            let e = &d.entries[i];
            (!is_dot_entry(&e.name)).then(|| e.path.clone())
        })
        .collect();
    if targets.is_empty() {
        ed.message("No files to copy");
        return Ok(());
    }
    let dir = ed.buf().dired().expect("dired buffer").dir.clone();
    ed.read_string(
        "Copy to: ",
        None,
        Box::new(move |ed, input| {
            if input.trim().is_empty() {
                return Ok(());
            }
            let dest_dir = PathBuf::from(input.trim());
            if !dest_dir.is_dir() {
                ed.error(format!("{} is not a directory", dest_dir.display()));
                return Ok(());
            }
            for src in &targets {
                let name = src.file_name().map(|s| s.to_os_string().to_owned());
                if let Some(name) = name {
                    if let Err(e) = copy_recursive(src, &dest_dir.join(name)) {
                        ed.error(format!("cannot copy {}: {e}", src.display()));
                    }
                }
            }
            // refresh the original directory listing
            let idx = ed.selected_buffer_index();
            let _ = idx;
            open_dir(ed, &dir, false)
        }),
    );
    Ok(())
}

fn cmd_dired_create_directory(ed: &mut Editor) -> Result<()> {
    let dir = ed.buf().dired().expect("dired buffer").dir.clone();
    ed.read_string(
        "Create directory: ",
        None,
        Box::new(move |ed, input| {
            if input.trim().is_empty() {
                return Ok(());
            }
            let path = dir.join(input.trim());
            if let Err(e) = fs::create_dir(&path) {
                ed.error(format!("cannot create {}: {e}", path.display()));
                return Ok(());
            }
            refresh(ed)
        }),
    );
    Ok(())
}

/// Register dired-mode and all dired commands; called from
/// `commands::register_defaults`.
pub fn register(ed: &mut Editor) {
    let mut km = Keymap::new();
    let b = |km: &mut Keymap, seq: &str, cmd: &str| {
        km.bind_sequence(&crate::key::parse_sequence(seq).unwrap(), cmd);
    };
    b(&mut km, "RET", "dired-open");
    b(&mut km, "f", "dired-open");
    b(&mut km, "o", "dired-open-other-window");
    b(&mut km, "^", "dired-up-directory");
    b(&mut km, "g", "dired-refresh");
    b(&mut km, "q", "dired-quit");
    b(&mut km, "m", "dired-mark");
    b(&mut km, "u", "dired-unmark");
    b(&mut km, "U", "dired-unmark-all");
    b(&mut km, "D", "dired-delete");
    b(&mut km, "R", "dired-rename");
    b(&mut km, "C", "dired-copy");
    b(&mut km, "+", "dired-create-directory");
    ed.register_mode_def(ModeDef {
        name: "dired-mode".into(),
        lang: None,
        indent_unit: None,
        comment_prefix: None,
        keymap: Some(km),
    });

    let add = |ed: &mut Editor, name: &str, doc: &'static str, f: fn(&mut Editor) -> Result<()>| {
        ed.commands_mut().add(name, doc, f);
    };
    add(ed, "dired", "Open a directory in dired.", cmd_dired);
    add(
        ed,
        "dired-open",
        "Open the entry under point.",
        cmd_dired_open,
    );
    add(
        ed,
        "dired-open-other-window",
        "Open the entry under point in another window.",
        cmd_dired_open_other_window,
    );
    add(
        ed,
        "dired-up-directory",
        "Go to the parent directory.",
        cmd_dired_up_directory,
    );
    add(
        ed,
        "dired-refresh",
        "Re-read the directory listing.",
        cmd_dired_refresh,
    );
    add(ed, "dired-quit", "Kill the dired buffer.", cmd_dired_quit);
    add(
        ed,
        "dired-mark",
        "Mark the entry under point.",
        cmd_dired_mark,
    );
    add(
        ed,
        "dired-unmark",
        "Unmark the entry under point.",
        cmd_dired_unmark,
    );
    add(
        ed,
        "dired-unmark-all",
        "Unmark all entries.",
        cmd_dired_unmark_all,
    );
    add(
        ed,
        "dired-delete",
        "Delete marked (or current) files.",
        cmd_dired_delete,
    );
    add(
        ed,
        "dired-rename",
        "Rename the entry under point.",
        cmd_dired_rename,
    );
    add(
        ed,
        "dired-copy",
        "Copy marked (or current) files.",
        cmd_dired_copy,
    );
    add(
        ed,
        "dired-create-directory",
        "Create a directory.",
        cmd_dired_create_directory,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("dired-test-{}-{n}", std::process::id()));
            fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn dired_ed(dir: &Path) -> Editor {
        let mut ed = Editor::new(20, 80);
        open_dir(&mut ed, dir, false).unwrap();
        ed
    }

    #[test]
    fn listing_sorted_dirs_first() {
        let t = TmpDir::new();
        fs::write(t.0.join("b.txt"), "x").unwrap();
        fs::create_dir(t.0.join("adir")).unwrap();
        fs::write(t.0.join("a.txt"), "x").unwrap();
        let ed = dired_ed(&t.0);
        let d = ed.buf().dired().unwrap();
        let names: Vec<&str> = d.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec![".", "..", "adir", "a.txt", "b.txt"]);
        let text = ed.buf().rope().to_string();
        assert!(text.starts_with(&format!("  {}:\n", t.0.display())));
        assert!(text.contains("adir/"));
    }

    #[test]
    fn marks_carry_across_refresh() {
        let t = TmpDir::new();
        fs::write(t.0.join("f1"), "x").unwrap();
        fs::write(t.0.join("f2"), "x").unwrap();
        let mut ed = dired_ed(&t.0);
        // point to the f1 line: entries are ., .., f1, f2
        let buf = ed.buf_mut();
        buf.move_to_line(4); // header 2 + f1 index 2
        let line = buf.line_of_point();
        let line_start = buf.rope().line_to_char(line);
        buf.set_point(line_start);
        cmd_dired_mark(&mut ed).unwrap();
        let text = ed.buf().rope().to_string();
        assert!(text.lines().nth(4).unwrap().starts_with('*'));
        refresh(&mut ed).unwrap();
        assert!(ed.buf().dired().unwrap().entries[2].marked);
        assert!(ed
            .buf()
            .rope()
            .to_string()
            .lines()
            .nth(4)
            .unwrap()
            .starts_with('*'));
    }

    #[test]
    fn entry_at_point_maps_lines() {
        let t = TmpDir::new();
        fs::write(t.0.join("file"), "x").unwrap();
        let mut ed = dired_ed(&t.0);
        // line 2 = ".", line 3 = "..", line 4 = "file"
        ed.buf_mut().move_to_line(4);
        assert_eq!(entry_at_point(&ed), Some(2));
        ed.buf_mut().move_to_line(0);
        assert_eq!(entry_at_point(&ed), None);
    }

    #[test]
    fn rename_entry() {
        let t = TmpDir::new();
        fs::write(t.0.join("old.txt"), "x").unwrap();
        let mut ed = dired_ed(&t.0);
        let buf = ed.buf_mut();
        buf.move_to_line(4);
        let line_start = buf.rope().line_to_char(4);
        buf.set_point(line_start);
        // answer the rename prompt directly
        let src = ed.buf().dired().unwrap().entries[2].path.clone();
        let dest = src.parent().unwrap().join("new.txt");
        fs::rename(&src, &dest).unwrap();
        refresh(&mut ed).unwrap();
        assert!(ed
            .buf()
            .dired()
            .unwrap()
            .entries
            .iter()
            .any(|e| e.name == "new.txt"));
    }

    #[test]
    fn open_nested_directory() {
        let t = TmpDir::new();
        fs::create_dir(t.0.join("sub")).unwrap();
        let mut ed = dired_ed(&t.0);
        let buf = ed.buf_mut();
        buf.move_to_line(4); // "sub" is the first entry after . and ..
        let line_start = buf.rope().line_to_char(4);
        buf.set_point(line_start);
        cmd_dired_open(&mut ed).unwrap();
        assert_eq!(
            ed.buf().dired().unwrap().dir,
            t.0.join("sub").canonicalize().unwrap()
        );
    }

    #[test]
    fn file_completion_lists_dir_entries() {
        let t = TmpDir::new();
        fs::create_dir(t.0.join("sub")).unwrap();
        fs::write(t.0.join("alpha.txt"), "a").unwrap();
        fs::write(t.0.join("beta.txt"), "b").unwrap();
        fs::write(t.0.join(".hidden"), "h").unwrap();
        let ed = Editor::new(20, 80);
        let base = t.0.display().to_string();
        assert_eq!(
            complete_file_names(&ed, &format!("{base}/a")),
            vec![format!("{base}/alpha.txt")]
        );
        // empty prefix: all entries, directories first, dotfiles hidden
        assert_eq!(
            complete_file_names(&ed, &format!("{base}/")),
            vec![
                format!("{base}/sub/"),
                format!("{base}/alpha.txt"),
                format!("{base}/beta.txt"),
            ]
        );
        // dotfiles appear once the prefix starts with '.'
        assert_eq!(
            complete_file_names(&ed, &format!("{base}/.")),
            vec![format!("{base}/.hidden")]
        );
        // non-existent directory: no candidates
        assert!(complete_file_names(&ed, &format!("{base}/nope/x")).is_empty());
    }

    #[test]
    fn file_completion_uses_buffer_file_parent_as_base() {
        let t = TmpDir::new();
        fs::write(t.0.join("alpha.txt"), "a").unwrap();
        fs::create_dir(t.0.join("sub")).unwrap();
        let mut ed = Editor::new(20, 80);
        // current buffer is a file inside t.0
        let buf = Buffer::load_file(t.0.join("alpha.txt")).unwrap();
        let id = buf.id;
        ed.add_buffer(buf);
        ed.set_selected_buffer(id);
        let cands = complete_file_names(&ed, "su");
        assert_eq!(cands, vec!["sub/"], "relative names from the file's dir");
        let cands = complete_file_names(&ed, "a");
        assert_eq!(cands, vec!["alpha.txt"]);
    }

    #[test]
    fn file_completion_uses_dired_dir_as_base() {
        let t = TmpDir::new();
        fs::write(t.0.join("alpha.txt"), "a").unwrap();
        let ed = dired_ed(&t.0);
        let cands = complete_file_names(&ed, "al");
        assert_eq!(cands, vec!["alpha.txt"]);
    }
}
