//! RowWalker must not recurse per skipped line.
use emacs_core::buffer::Buffer;
use emacs_core::wrap::RowWalker;

#[test]
fn walker_skips_many_lines_without_overflowing() {
    let mut b = Buffer::new("test");
    for i in 0..200_000 {
        b.insert(&format!("line {i}\n"));
    }
    let mut w = RowWalker::new(&b, 80, 199_999);
    let (line, seg, s, e) = w.next_row().expect("a row");
    assert_eq!(line, 199_999);
    assert_eq!(seg, 0);
    assert_eq!(s, 0);
    assert!(e > 0);
}
