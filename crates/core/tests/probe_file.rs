#[test]
fn probe_rows_between_on_file() {
    let mut b = emacs_core::buffer::Buffer::new("t");
    let content =
        std::fs::read_to_string("/build/llvm-dev/build/lib/Target/AMDGPU/AMDGPUGenGlobalISel.inc")
            .unwrap();
    b.insert(&content);
    let t = std::time::Instant::now();
    let rows = emacs_core::wrap::rows_between(&b, 80, 0, b.rope().len_lines());
    println!("rows_between(0, len) w80 = {rows} ({:?})", t.elapsed());
    // also count with row_count per line (the walk's path)
    let t = std::time::Instant::now();
    let mut total = 0usize;
    for l in 0..b.rope().len_lines() {
        total += emacs_core::wrap::row_count(b.line(l), 80);
    }
    println!("sum row_count per line = {total} ({:?})", t.elapsed());
}
