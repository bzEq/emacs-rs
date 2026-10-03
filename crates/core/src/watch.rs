//! File watching for auto-revert: a core facility that watches the parent
//! directory of every buffer file (editors often replace files via rename,
//! which breaks watches on the file itself) and reports events for watched
//! files. The revert policy lives in Lua.
//!
//! This is Emacs's file-notification layer in miniature (`inotify.c` /
//! `file-notify.c`); the event loop in `emacs-app` owns the instance and
//! calls [`FileWatcher::sync`] / [`FileWatcher::drain`].

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use notify::Watcher;

use crate::editor::Editor;

/// The small part of a platform watcher needed by [`FileWatcher`]. Keeping
/// the set reconciliation independent of notify makes retry behavior
/// deterministic to test.
trait WatchOps {
    fn watch_dir(&mut self, path: &Path) -> notify::Result<()>;
    fn unwatch_dir(&mut self, path: &Path) -> notify::Result<()>;
}

struct NotifyOps {
    watcher: notify::RecommendedWatcher,
}

impl WatchOps for NotifyOps {
    fn watch_dir(&mut self, path: &Path) -> notify::Result<()> {
        self.watcher
            .watch(path, notify::RecursiveMode::NonRecursive)
    }

    fn unwatch_dir(&mut self, path: &Path) -> notify::Result<()> {
        self.watcher.unwatch(path)
    }
}

/// Reconcile a set of directory watches, retaining only successful changes.
/// Failed operations deliberately remain out of (or in) `watched_dirs`, so a
/// later call retries them.
fn sync_watches<O: WatchOps>(
    ops: &mut O,
    watched_dirs: &mut HashSet<PathBuf>,
    desired_dirs: &HashSet<PathBuf>,
) {
    let to_watch: Vec<PathBuf> = desired_dirs.difference(watched_dirs).cloned().collect();
    for dir in to_watch {
        if ops.watch_dir(&dir).is_ok() {
            watched_dirs.insert(dir);
        }
    }

    let to_unwatch: Vec<PathBuf> = watched_dirs.difference(desired_dirs).cloned().collect();
    for dir in to_unwatch {
        if ops.unwatch_dir(&dir).is_ok() {
            watched_dirs.remove(&dir);
        }
    }
}

/// Make an absolute path without resolving symlinks or requiring the final
/// path to exist.
fn absolute_lexical(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };

    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => result.push(prefix.as_os_str()),
            Component::RootDir => result.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                // Never pop a root/prefix. This also keeps the function
                // lexical when an unresolved path contains `..`.
                if result.file_name().is_some() {
                    result.pop();
                }
            }
            Component::Normal(part) => result.push(part),
        }
    }
    result
}

/// Normalize an event/request path while preserving a useful identity when
/// the file itself is absent. Existing paths are fully canonicalized. For a
/// missing path, its absolute lexical spelling is retained, while the
/// nearest existing ancestor is canonicalized (including symlink parents).
fn normalize_path(path: &Path) -> PathBuf {
    let absolute = absolute_lexical(path);
    if let Ok(canonical) = std::fs::canonicalize(&absolute) {
        return canonical;
    }

    let mut ancestor = absolute.as_path();
    while let Some(parent) = ancestor.parent() {
        if let Ok(canonical_parent) = std::fs::canonicalize(ancestor) {
            if let Ok(suffix) = absolute.strip_prefix(ancestor) {
                return canonical_parent.join(suffix);
            }
        }
        ancestor = parent;
    }
    absolute
}

pub struct FileWatcher {
    watcher: Option<NotifyOps>,
    rx: std::sync::mpsc::Receiver<Result<notify::Event, notify::Error>>,
    /// Directories successfully registered with the platform watcher.
    watched_dirs: HashSet<PathBuf>,
    /// Normalized file path -> buffer ids showing it.
    files: HashMap<PathBuf, Vec<usize>>,
}

impl FileWatcher {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let watcher = notify::recommended_watcher(tx)
            .map(|watcher| NotifyOps { watcher })
            .ok();
        if watcher.is_none() {
            eprintln!("em: warning: file watching unavailable; auto-revert disabled");
        }
        FileWatcher {
            watcher,
            rx,
            watched_dirs: HashSet::new(),
            files: HashMap::new(),
        }
    }

    /// Match the watch set to the editor's buffers: watch the parent
    /// directory of every buffer with a file, stop watching stale ones.
    pub fn sync(&mut self, ed: &Editor) {
        let mut files: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        let mut dirs: HashSet<PathBuf> = HashSet::new();
        for buf in ed.buffers() {
            let Some(path) = buf.path() else { continue };
            let normalized = normalize_path(path);
            files.entry(normalized.clone()).or_default().push(buf.id);
            if let Some(dir) = normalized.parent() {
                dirs.insert(dir.to_path_buf());
            }
        }
        if let Some(ops) = self.watcher.as_mut() {
            sync_watches(ops, &mut self.watched_dirs, &dirs);
        }
        self.files = files;
    }

    /// Drain pending file events; return the ids of buffers whose files
    /// changed, deduplicated. Event paths use the same normalization as
    /// requested buffer paths, which handles delete/recreate and rename/
    /// replacement notifications consistently across platforms.
    pub fn drain(&self) -> Vec<usize> {
        let mut changed: Vec<usize> = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            let Ok(event) = event else { continue };
            for path in &event.paths {
                let normalized = normalize_path(path);
                if let Some(ids) = self.files.get(&normalized) {
                    for &id in ids {
                        if !changed.contains(&id) {
                            changed.push(id);
                        }
                    }
                }
            }
        }
        changed
    }
}

impl Default for FileWatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use std::collections::VecDeque;
    use std::fs;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    struct FakeOps {
        failures: VecDeque<bool>,
        calls: Vec<(bool, PathBuf)>,
    }

    impl WatchOps for FakeOps {
        fn watch_dir(&mut self, path: &Path) -> notify::Result<()> {
            self.calls.push((true, path.to_path_buf()));
            if self.failures.pop_front().unwrap_or(false) {
                Err(notify::Error::generic("injected watch failure"))
            } else {
                Ok(())
            }
        }

        fn unwatch_dir(&mut self, path: &Path) -> notify::Result<()> {
            self.calls.push((false, path.to_path_buf()));
            if self.failures.pop_front().unwrap_or(false) {
                Err(notify::Error::generic("injected unwatch failure"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn failed_watch_and_unwatch_are_retried() {
        let dir = PathBuf::from("/tmp/watch-retry-test");
        let mut desired = HashSet::from([dir.clone()]);
        let mut watched = HashSet::new();
        let mut ops = FakeOps {
            failures: VecDeque::from([true, false]),
            calls: Vec::new(),
        };

        sync_watches(&mut ops, &mut watched, &desired);
        assert!(watched.is_empty());
        sync_watches(&mut ops, &mut watched, &desired);
        assert_eq!(watched, HashSet::from([dir.clone()]));

        desired.clear();
        ops.failures = VecDeque::from([true, false]);
        sync_watches(&mut ops, &mut watched, &desired);
        assert_eq!(watched, HashSet::from([dir.clone()]));
        sync_watches(&mut ops, &mut watched, &desired);
        assert!(watched.is_empty());
        assert_eq!(ops.calls.len(), 4);
    }

    #[cfg(unix)]
    #[test]
    fn missing_file_keeps_absolute_name_with_canonical_parent() {
        let root = unique_temp_dir("missing");
        fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        let requested = root.join("link").join("not-yet-there");
        let normalized = normalize_path(&requested);
        let expected = fs::canonicalize(root.join("real"))
            .unwrap()
            .join("not-yet-there");
        assert_eq!(normalized, expected);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_recreate_and_atomic_replace_report_the_buffer() {
        let root = unique_temp_dir("events");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("watched.txt");
        fs::write(&path, "one").unwrap();

        let mut ed = Editor::new(20, 80);
        let mut buffer = Buffer::new("watched.txt");
        let id = buffer.id;
        buffer.set_path(Some(path.clone()));
        ed.add_buffer(buffer);
        let mut watcher = FileWatcher::new();
        watcher.sync(&ed);

        fs::remove_file(&path).unwrap();
        watcher.sync(&ed);
        fs::write(&path, "two").unwrap();
        assert!(wait_for_change(&watcher, id));

        let replacement = root.join("replacement.tmp");
        fs::write(&replacement, "three").unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert!(wait_for_change(&watcher, id));
        let _ = fs::remove_dir_all(root);
    }

    fn wait_for_change(watcher: &FileWatcher, id: usize) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if watcher.drain().contains(&id) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("emacs-rs-watch-{label}-{nanos}"))
    }
}
