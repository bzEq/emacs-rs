//! Shared PTY test harness: spawns the `em` binary in a pseudo-terminal,
//! feeds it keystrokes, and reconstructs the screen for assertions.
//!
//! Tests use the real binary (`CARGO_BIN_EXE_em`), an isolated
//! `XDG_CONFIG_HOME`, and poll the reconstructed screen instead of relying
//! on fixed sleeps, so they are deterministic across machines.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Minimal terminal emulator: interprets the cursor/clear escape sequences
/// `em` emits and tracks the visible cells.
pub struct Screen {
    rows: usize,
    cols: usize,
    cells: Vec<Vec<char>>,
    row: usize,
    col: usize,
}

impl Screen {
    fn new(rows: u16, cols: u16) -> Self {
        Screen {
            rows: rows as usize,
            cols: cols as usize,
            cells: vec![vec![' '; cols as usize]; rows as usize],
            row: 0,
            col: 0,
        }
    }

    pub fn feed(&mut self, data: &[u8]) {
        let mut i = 0;
        while i < data.len() {
            let b = data[i];
            if b == 0x1b {
                if i + 1 < data.len() && (data[i + 1] == b'(' || data[i + 1] == b')') {
                    i += 2;
                    continue;
                }
                if i + 1 < data.len() && data[i + 1] == b'[' {
                    let mut j = i + 2;
                    while j < data.len() && !(0x40..=0x7e).contains(&data[j]) {
                        j += 1;
                    }
                    if j < data.len() {
                        let final_byte = data[j];
                        if final_byte == b'H' || final_byte == b'f' {
                            let params = String::from_utf8_lossy(&data[i + 2..j]);
                            let parts: Vec<&str> = params.split(';').collect();
                            let r = parts
                                .first()
                                .and_then(|x| x.parse::<usize>().ok())
                                .unwrap_or(1)
                                .saturating_sub(1);
                            let c = parts
                                .get(1)
                                .and_then(|x| x.parse::<usize>().ok())
                                .unwrap_or(1)
                                .saturating_sub(1);
                            self.row = r.min(self.rows - 1);
                            self.col = c.min(self.cols - 1);
                        }
                        // 'm', 'J', 'h', 'l', ...: cosmetic, ignored
                        i = j + 1;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            if b == b'\r' {
                self.col = 0;
                i += 1;
                continue;
            }
            if b == b'\n' {
                self.row = (self.row + 1).min(self.rows - 1);
                i += 1;
                continue;
            }
            if b >= 0x20 {
                let len = utf8_len(b);
                if i + len <= data.len() {
                    if let Ok(s) = std::str::from_utf8(&data[i..i + len]) {
                        if let Some(c) = s.chars().next() {
                            if self.row < self.rows && self.col < self.cols {
                                self.cells[self.row][self.col] = c;
                            }
                            self.col += 1;
                        }
                    }
                }
                i += len;
                continue;
            }
            i += 1;
        }
    }

    /// The whole screen as text, one row per line (trailing blanks trimmed).
    pub fn text(&self) -> String {
        self.cells
            .iter()
            .map(|r| r.iter().collect::<String>())
            .map(|s| s.trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn row_text(&self, row: usize) -> String {
        self.cells[row]
            .iter()
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    pub fn contains(&self, needle: &str) -> bool {
        self.text().contains(needle)
    }
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >= 0xf0 {
        4
    } else if b >= 0xe0 {
        3
    } else {
        2
    }
}

/// Spawn the editor under a PTY and drive it.
pub struct Em {
    /// Per-test scratch directory: isolated XDG config + test files.
    pub scratch: PathBuf,
    pub screen: Screen,
    /// Everything the terminal received so far.
    pub raw: Vec<u8>,
    fed: usize,
    master: File,
    child: Child,
    exited: Option<ExitStatus>,
    /// Whether this instance owns the scratch dir (removes it on drop).
    remove_scratch: bool,
}

fn unique_scratch() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("em-pty-test-{}-{n}", std::process::id()));
    std::fs::create_dir_all(p.join("cfg")).unwrap();
    p
}

/// A fresh scratch directory for one test (config + files).
pub fn scratch_dir() -> PathBuf {
    unique_scratch()
}

/// Locate the `em` binary. Cargo sets `CARGO_BIN_EXE_em` when all tests run;
/// when a single test target is selected with `--test`, it does not build
/// the binary, so fall back to the target directory layout.
fn em_binary() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_em") {
        return PathBuf::from(p);
    }
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "debug".into());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join(profile)
        .join("em");
    assert!(
        path.exists(),
        "em binary not found at {path:?}; run `cargo build` or `cargo test` first"
    );
    path
}

impl Em {
    pub fn spawn() -> Self {
        Self::spawn_with_args(&[])
    }

    /// Spawn `em` with the given arguments; `XDG_CONFIG_HOME` points at a
    /// per-test scratch dir so the developer's real config never leaks in.
    pub fn spawn_with_args(args: &[&str]) -> Self {
        let mut em = Self::spawn_with_scratch(unique_scratch(), args);
        em.remove_scratch = true;
        em
    }

    /// Spawn `em` using an existing scratch dir (so the caller can seed an
    /// init.lua and files first). The caller keeps ownership of the dir.
    pub fn spawn_with_scratch(scratch: PathBuf, args: &[&str]) -> Self {
        let bin = em_binary();
        let (master, slave) = openpty(24, 80);
        let child = Command::new(&bin)
            .args(args)
            .env("XDG_CONFIG_HOME", scratch.join("cfg"))
            .env("TERM", "xterm-256color")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave))
            .spawn()
            .expect("failed to spawn em");
        set_nonblocking(&master);
        Em {
            scratch,
            screen: Screen::new(24, 80),
            raw: Vec::new(),
            fed: 0,
            master,
            child,
            exited: None,
            remove_scratch: false,
        }
    }

    /// The scratch dir path as a string (for passing to `em`).
    pub fn scratch_str(&self) -> String {
        self.scratch.to_string_lossy().into_owned()
    }

    /// Send raw key bytes, then drain output.
    pub fn keys(&mut self, bytes: &[u8]) {
        let _ = self.master.write_all(bytes);
        std::thread::sleep(Duration::from_millis(60));
        self.drain();
    }

    /// Type a string (each byte becomes a key press).
    pub fn type_str(&mut self, s: &str) {
        self.keys(s.as_bytes());
    }

    /// M-x <command> RET helper.
    pub fn m_x(&mut self, command: &str) {
        self.keys(b"\x1bx");
        self.drain();
        self.type_str(command);
        self.keys(b"\r");
    }

    /// Read pending output and update the screen model.
    pub fn drain(&mut self) {
        let mut buf = [0u8; 8192];
        loop {
            match self.master.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.raw.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        self.screen.feed(&self.raw[self.fed..]);
        self.fed = self.raw.len();
    }

    /// Poll until the screen contains `text`; returns false on timeout or
    /// child exit.
    pub fn wait_for(&mut self, text: &str, timeout_ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            self.drain();
            if self.screen.contains(text) {
                return true;
            }
            if self.child.try_wait().ok().flatten().is_some() {
                self.drain();
                return self.screen.contains(text);
            }
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    /// Wait until a screen row contains `text` (whole-row containment).
    pub fn wait_for_row(&mut self, row: usize, text: &str, timeout_ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        loop {
            self.drain();
            if self.screen.row_text(row).contains(text) {
                return true;
            }
            if self.child.try_wait().ok().flatten().is_some() {
                return false;
            }
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    pub fn raw_contains(&self, needle: &[u8]) -> bool {
        self.raw.windows(needle.len()).any(|w| w == needle)
    }

    /// Quit the editor: C-x C-c, answering `n` to any confirmation prompts.
    pub fn quit(&mut self) {
        self.keys(b"\x18\x03");
        for _ in 0..10 {
            self.drain();
            if self.child.try_wait().ok().flatten().is_some() {
                self.exited = self.child.wait().ok();
                return;
            }
            let _ = self.master.write_all(b"n");
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
        self.exited = self.child.wait().ok();
    }

    /// Exit status once the process has ended (waits up to 5s for it).
    pub fn exit_status(&mut self) -> Option<ExitStatus> {
        if self.exited.is_none() {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                self.drain();
                match self.child.try_wait() {
                    Ok(Some(status)) => {
                        self.exited = Some(status);
                        break;
                    }
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    _ => {
                        let _ = self.child.kill();
                        self.exited = self.child.wait().ok();
                        break;
                    }
                }
            }
        }
        self.exited
    }
}

impl Drop for Em {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if self.remove_scratch {
            let _ = std::fs::remove_dir_all(&self.scratch);
        }
    }
}

fn openpty(rows: u16, cols: u16) -> (File, File) {
    unsafe {
        let mut master: libc::c_int = -1;
        let mut slave: libc::c_int = -1;
        let mut ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let r = libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &mut ws,
        );
        assert_eq!(r, 0, "openpty failed: {}", std::io::Error::last_os_error());
        (File::from_raw_fd(master), File::from_raw_fd(slave))
    }
}

fn set_nonblocking(f: &File) {
    use std::os::fd::AsRawFd;
    unsafe {
        let fd = f.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}

/// Helper: write a file into `dir`.
pub fn write_file(dir: &Path, name: &str, content: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, content).unwrap();
    p
}

/// Helper: write an init.lua into the scratch config dir.
pub fn write_init(scratch: &Path, lua: &str) {
    let cfg = scratch.join("cfg").join("emacs-rs");
    std::fs::create_dir_all(&cfg).unwrap();
    std::fs::write(cfg.join("init.lua"), lua).unwrap();
}
