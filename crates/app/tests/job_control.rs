//! C-z (`suspend-frame`): stop the process and hand the terminal back to
//! the shell, like a foreground job; `fg` (SIGCONT) resumes it.

mod common;

use common::{write_file, Em};

/// True if the child is in a job-control stop (`T` in its stat file).
fn is_stopped(pid: u32) -> bool {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    let state = stat.rsplit(')').next().unwrap_or("");
    state.split_whitespace().next() == Some("T")
}

#[test]
fn suspend_stops_and_resume_redraws() {
    // C-z must stop the process (the terminal is left in a usable state
    // for the shell), and SIGCONT must bring the editor back with the
    // screen redrawn.
    let em = Em::spawn();
    let content: String = (0..60).map(|i| format!("line {i}\n")).collect();
    let path = write_file(&em.scratch, "t.txt", &content);
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_job(em.scratch.clone(), &[&path_s]);
    assert!(em.wait_for("line 0", 5000));
    let pid = em.pid();

    em.keys(b"\x1a"); // C-z
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !is_stopped(pid) {
        assert!(
            std::time::Instant::now() < deadline,
            "em did not stop on C-z"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    // resume it the way the shell's `fg` does
    unsafe {
        libc::kill(pid as i32, libc::SIGCONT);
    }
    assert!(em.wait_for("line 0", 5000), "screen redrawn after resume");
    em.quit();
}
