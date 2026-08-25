//! Debug: print every event crossterm receives.

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

fn main() {
    enable_raw_mode().unwrap();
    eprintln!("raw mode on");
    let start = std::time::Instant::now();
    loop {
        match event::read().unwrap() {
            Event::Key(k) => {
                eprintln!(
                    "{:?} key: code={:?} mods={:?} kind={:?}",
                    start.elapsed(),
                    k.code,
                    k.modifiers,
                    k.kind
                );
                if k.kind == KeyEventKind::Press
                    && k.code == KeyCode::Char('q')
                    && k.modifiers.contains(KeyModifiers::CONTROL)
                {
                    break;
                }
            }
            Event::Resize(w, h) => eprintln!("resize {w}x{h}"),
            other => eprintln!("event: {other:?}"),
        }
    }
    disable_raw_mode().unwrap();
    eprintln!("done");
}
