//! File watching for auto-revert: a core facility that watches the parent
//! directory of every buffer file (editors often replace files via rename,
//! which breaks watches on the file itself) and reports events for watched
//! files. The revert policy lives in Lua.
//!
//! This is Emacs's file-notification layer in miniature (`inotify.c` /
//! `file-notify.c`); the event loop in `emacs-app` owns the instance and
//! calls [`FileWatcher::sync`] / [`FileWatcher::drain`].

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use notify::Watcher;

use crate::editor::Editor;

pub struct FileWatcher {
    watcher: Option<notify::RecommendedWatcher>,
    rx: std::sync::mpsc::Receiver<Result<notify::Event, notify::Error>>,
    watched_dirs: HashSet<PathBuf>,
    /// canonical file path -> buffer ids showing it
    files: HashMap<PathBuf, Vec<usize>>,
}

impl FileWatcher {
    pub fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let watcher = notify::recommended_watcher(tx).ok();
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
            let Ok(canon) = std::fs::canonicalize(path) else {
                continue;
            };
            files.entry(canon.clone()).or_default().push(buf.id);
            if let Some(dir) = canon.parent() {
                dirs.insert(dir.to_path_buf());
            }
        }
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        for dir in dirs.difference(&self.watched_dirs) {
            let _ = watcher.watch(dir, notify::RecursiveMode::NonRecursive);
        }
        for dir in self.watched_dirs.difference(&dirs) {
            let _ = watcher.unwatch(dir);
        }
        self.watched_dirs = dirs;
        self.files = files;
    }

    /// Drain pending inotify events; return the ids of buffers whose files
    /// changed, deduplicated.
    pub fn drain(&self) -> Vec<usize> {
        let mut changed: Vec<usize> = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            let Ok(event) = event else { continue };
            for path in &event.paths {
                if let Some(ids) = self.files.get(path) {
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
