//! Large-file benchmark for the rope-backed buffer.
//!
//! Usage:
//!   cargo run -p emacs-core --example bench -- gen <path> <size_mb>
//!   cargo run -p emacs-core --example bench -- load <path>

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use emacs_core::buffer::{Buffer, Direction};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gen") => {
            let path = &args[2];
            let mb: usize = args[3].parse().unwrap();
            gen(path, mb);
        }
        Some("load") => load(Path::new(&args[2])),
        _ => eprintln!("usage: bench gen <path> <mb> | bench load <path>"),
    }
}

fn gen(path: &str, mb: usize) {
    let line = "2026-08-25T12:00:00.000Z INFO  app::module: this is a benchmark log line with some padding content here\n";
    let mut w = BufWriter::new(File::create(path).unwrap());
    let mut written = 0usize;
    while written < mb * 1024 * 1024 {
        w.write_all(line.as_bytes()).unwrap();
        written += line.len();
    }
    w.flush().unwrap();
    println!("generated {} ({} MB)", path, mb);
}

fn load(path: &Path) {
    let size = std::fs::metadata(path).unwrap().len();
    println!("file: {} ({:.1} MB)", path.display(), size as f64 / 1e6);

    let t = Instant::now();
    let file = File::open(path).unwrap();
    let mut buf = Buffer::from_reader("bench", file).unwrap();
    println!(
        "load via Rope::from_reader: {:?} ({:.0} MB/s)",
        t.elapsed(),
        size as f64 / 1e6 / t.elapsed().as_secs_f64()
    );

    let rope = buf.rope();
    let lines = rope.len_lines();
    let orig_len = buf.len_chars();

    let t = Instant::now();
    let mut sum = 0usize;
    for _ in 0..1_000_000 {
        sum = sum.wrapping_add(rope.line_to_char(sum % lines));
    }
    println!("1M random line_to_char: {:?}", t.elapsed());

    let t = Instant::now();
    let mut sum = 0usize;
    for _ in 0..1_000_000 {
        sum = sum.wrapping_add(rope.char_to_line(sum % rope.len_chars()));
    }
    println!("1M random char_to_line: {:?}", t.elapsed());

    let t = Instant::now();
    let n = 100_000;
    for i in 0..n {
        let pos = (i * 7919) % buf.len_chars();
        buf.set_point(pos);
        buf.insert_char('x');
    }
    println!("100k random single-char inserts: {:?}", t.elapsed());

    let t = Instant::now();
    for _ in 0..n {
        buf.move_char(Direction::Backward);
        buf.delete_forward();
    }
    println!("100k deletes: {:?}", t.elapsed());
    assert_eq!(buf.len_chars(), orig_len);
}
