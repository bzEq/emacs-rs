//! PTY regression tests: window splits (C-x 2/3/0/1/o) and isearch.

mod common;

use common::{write_file, Em};

#[test]
fn split_vertical_shows_two_panes() {
    let mut em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "hello world\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("hello world", 5000));
    em.keys(b"\x18\x00"); // C-x 2 (Ctrl+2 byte)
                          // wait until the buffer text appears in both panes
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        em.drain();
        if em.screen.text().matches("hello world").count() >= 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "buffer visible in both panes"
        );
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    em.quit();
}

#[test]
fn split_and_delete_other_windows() {
    let mut em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "hello world\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("hello world", 5000));
    em.keys(b"\x18\x00"); // C-x 2
    assert!(wait_for_count(&mut em, 2), "two windows");
    em.keys(b"\x18\x1b"); // C-x 3 (Ctrl+3 = ESC byte)
    assert!(wait_for_count(&mut em, 3), "three windows");
    em.keys(b"\x18o"); // C-x o
    em.keys(b"\x18\x31"); // C-x 1
    assert!(wait_for_count(&mut em, 1), "back to a single window");
    em.quit();
}

/// Poll until "hello world" appears exactly `n` times on screen.
fn wait_for_count(em: &mut Em, n: usize) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        em.drain();
        if em.screen.text().matches("hello world").count() == n {
            return true;
        }
        if std::time::Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
}

#[test]
fn isearch_finds_match() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "hello world\nsecond line\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("hello world", 5000));
    em.keys(b"\x13"); // C-s
    em.type_str("line");
    assert!(em.wait_for("I-search: line", 3000), "isearch prompt");
    assert!(em.wait_for("L2 C7", 3000), "point at 'line' in line 2");
    em.keys(b"\r"); // accept
    assert!(em.wait_for("L2 C7", 3000), "point stays after accept");
    em.quit();
}

#[test]
fn isearch_failing_and_abort() {
    let em = Em::spawn();
    let path = write_file(&em.scratch, "t.txt", "abc\n");
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("abc", 5000));
    em.keys(b"\x13"); // C-s
    em.type_str("zzz");
    assert!(em.wait_for("Failing", 3000), "failing search shown");
    em.keys(b"\x07"); // C-g abort
    assert!(em.wait_for("L1 C0", 3000), "point back at start");
    em.quit();
}
