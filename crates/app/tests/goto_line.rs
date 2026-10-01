//! goto-line (M-g M-g) must not activate the mark: Emacs sets the mark
//! (C-x C-x can jump back) but leaves the region inactive, so nothing is
//! highlighted.

mod common;

use common::{write_file, Em};

#[test]
fn goto_line_does_not_highlight_the_region() {
    let em = Em::spawn();
    let content: String = (0..60).map(|i| format!("line {i}\n")).collect();
    let path = write_file(&em.scratch, "t.txt", &content);
    let path_s = path.to_string_lossy().into_owned();
    let mut em = Em::spawn_with_args(&[&path_s]);
    assert!(em.wait_for("line 0", 5000));

    em.keys(b"\x1bg\x1bg"); // M-g M-g: goto-line prompt
    em.type_str("40");
    em.keys(b"\r");
    assert!(em.wait_for("Goto line 40", 3000), "goto-line ran");
    assert!(em.wait_for("line 39", 3000), "view jumped to line 40");
    em.drain();

    // with an inactive mark the region background (48;5;12, LightBlue)
    // never appears
    assert!(
        !em.raw_contains(b"48;5;12m"),
        "goto-line must not activate the region"
    );

    // the mark is still set: C-x C-x exchanges point and mark (and only
    // then activates the region)
    em.keys(b"\x18\x18"); // C-x C-x
    assert!(
        em.wait_for("line 0", 3000),
        "exchange-point-and-mark jumps back to the old point"
    );
    em.drain();
    assert!(
        em.raw_contains(b"48;5;12m"),
        "exchange-point-and-mark activates the region"
    );
    em.quit();
}
