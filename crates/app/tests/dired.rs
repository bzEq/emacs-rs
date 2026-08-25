//! PTY regression tests: dired — listing, marking, rename, delete, mkdir,
//! and subdirectory navigation.

mod common;

use common::Em;

fn open_dired(em: &mut Em, dir: &str) {
    // wait until the editor has entered raw mode (initial frame drawn)
    // before sending keys, so the pty does not echo them back
    assert!(em.wait_for("lines", 5000), "editor ready");
    em.keys(b"\x18d"); // C-x d
    em.type_str(dir);
    em.keys(b"\r");
}

#[test]
fn listing_shows_entries() {
    let em = Em::spawn();
    let d = em.scratch.join("work");
    std::fs::create_dir(&d).unwrap();
    std::fs::write(d.join("alpha.txt"), "aaa").unwrap();
    std::fs::create_dir(d.join("sub")).unwrap();
    let d_s = d.to_string_lossy().into_owned();
    let mut em = Em::spawn();
    open_dired(&mut em, &d_s);
    assert!(em.wait_for("alpha.txt", 5000), "file listed");
    assert!(em.wait_for("sub/", 3000), "dir listed with slash");
    assert!(em.wait_for("(dired-mode)", 3000), "dired-mode active");
    assert!(em.wait_for("./", 3000), "dot entries present");
    em.quit();
}

#[test]
fn mark_and_rename() {
    let em = Em::spawn();
    let d = em.scratch.join("work");
    std::fs::create_dir(&d).unwrap();
    std::fs::write(d.join("alpha.txt"), "aaa").unwrap();
    let d_s = d.to_string_lossy().into_owned();
    let mut em = Em::spawn();
    open_dired(&mut em, &d_s);
    assert!(em.wait_for("alpha.txt", 5000));
    // entries: . .. alpha.txt -> alpha.txt is line 4 (header 2 + index 2)
    for _ in 0..4 {
        em.keys(b"\x0e"); // C-n
    }
    assert!(em.wait_for("L5 C0", 3000), "point on alpha.txt line");
    em.keys(b"m");
    assert!(em.wait_for_row(4, "*", 3000), "marked line gets a * prefix");
    em.keys(b"R");
    assert!(em.wait_for("Rename alpha.txt", 3000), "rename prompt");
    em.type_str("gamma.txt");
    em.keys(b"\r");
    assert!(em.wait_for("gamma.txt", 5000), "listing refreshed");
    assert!(
        !std::fs::exists(d.join("alpha.txt")).unwrap(),
        "old name gone"
    );
    assert!(std::fs::exists(d.join("gamma.txt")).unwrap());
    em.quit();
}

#[test]
fn delete_with_confirmation() {
    let em = Em::spawn();
    let d = em.scratch.join("work");
    std::fs::create_dir(&d).unwrap();
    std::fs::write(d.join("beta.txt"), "bbb").unwrap();
    let d_s = d.to_string_lossy().into_owned();
    let mut em = Em::spawn();
    open_dired(&mut em, &d_s);
    assert!(em.wait_for("beta.txt", 5000));
    for _ in 0..4 {
        em.keys(b"\x0e");
    }
    em.keys(b"D");
    assert!(
        em.wait_for("Delete beta.txt? (y/n)", 3000),
        "confirm prompt"
    );
    em.keys(b"y");
    assert!(
        em.wait_for("total 2", 5000),
        "listing refreshed after delete"
    );
    assert!(!std::fs::exists(d.join("beta.txt")).unwrap());
    em.quit();
}

#[test]
fn create_directory_and_navigate() {
    let em = Em::spawn();
    let d = em.scratch.join("work");
    std::fs::create_dir(&d).unwrap();
    std::fs::create_dir(d.join("sub")).unwrap();
    let d_s = d.to_string_lossy().into_owned();
    let mut em = Em::spawn();
    open_dired(&mut em, &d_s);
    assert!(em.wait_for("sub/", 5000));
    // sub is line 4 (., .., sub)
    for _ in 0..4 {
        em.keys(b"\x0e");
    }
    em.keys(b"\r"); // enter sub
    assert!(em.wait_for(&format!("{}/sub:", d_s), 5000), "inside subdir");
    em.keys(b"+");
    assert!(em.wait_for("Create directory:", 3000));
    em.type_str("newdir");
    em.keys(b"\r");
    assert!(em.wait_for("newdir/", 5000), "created dir listed");
    assert!(std::fs::exists(d.join("sub").join("newdir")).unwrap());
    em.quit();
}
