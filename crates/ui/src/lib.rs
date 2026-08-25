//! Terminal rendering: buffer window, echo area / minibuffer, cursor.

use emacs_core::editor::Editor;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as TuiLine, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub const TAB_WIDTH: usize = 8;

/// Line content excluding the trailing `\r`/`\n`.
fn visible_content(s: ropey::RopeSlice<'_>) -> ropey::RopeSlice<'_> {
    let len = s.len_chars();
    if len >= 2 && s.char(len - 1) == '\n' && s.char(len - 2) == '\r' {
        s.slice(..len - 2)
    } else if len >= 1 && s.char(len - 1) == '\n' {
        s.slice(..len - 1)
    } else {
        s
    }
}

fn visual_col(s: ropey::RopeSlice<'_>) -> usize {
    let mut col = 0usize;
    for c in s.chars() {
        col += if c == '\t' {
            TAB_WIDTH - col % TAB_WIDTH
        } else {
            1
        };
    }
    col
}

fn expand_tabs(s: ropey::RopeSlice<'_>) -> String {
    let mut col = 0usize;
    let mut out = String::with_capacity(s.len_chars());
    for c in s.chars() {
        if c == '\t' {
            let spaces = TAB_WIDTH - col % TAB_WIDTH;
            out.extend(std::iter::repeat_n(' ', spaces));
            col += spaces;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

/// Render the editor: buffer window plus the echo area (which doubles as the
/// minibuffer while it is active). Returns the on-screen cursor position.
pub fn render(frame: &mut Frame, ed: &Editor) -> Option<(u16, u16)> {
    let area = frame.area();
    if area.height == 0 {
        return None;
    }

    let body = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let echo_rect = Rect {
        y: area.y + body.height,
        height: area.height - body.height,
        ..area
    };

    // --- buffer window -----------------------------------------------------
    let buf = ed.buf();
    let view = ed.view();
    let lines: Vec<TuiLine> = (0..body.height as usize)
        .filter_map(|i| {
            let line_idx = view.top_line + i;
            (line_idx < buf.len_lines())
                .then(|| TuiLine::from(expand_tabs(visible_content(buf.line(line_idx)))))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), body);

    // --- echo area ---------------------------------------------------------
    let echo_style = if ed.echo_is_error() {
        Style::default().fg(Color::White).bg(Color::Red)
    } else {
        Style::default().fg(Color::Black).bg(Color::White)
    };
    let echo_text: String = if let Some(mb) = ed.minibuffer() {
        let caret = if mb.cursor == mb.input.chars().count() {
            "█"
        } else {
            ""
        };
        format!("{}{}{}", mb.prompt, mb.input, caret)
    } else if let Some(emacs_core::minibuffer::Pending::YesNo { prompt, .. }) = ed.pending() {
        prompt.clone()
    } else if let Some(msg) = ed.echo() {
        msg.to_string()
    } else {
        status_line(ed)
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            echo_text,
            echo_style.add_modifier(Modifier::BOLD),
        )),
        echo_rect,
    );

    // --- cursor ------------------------------------------------------------
    if let Some(mb) = ed.minibuffer() {
        // cursor in the minibuffer
        let x = (echo_rect.x as usize + mb.prompt.chars().count() + mb.cursor)
            .min((echo_rect.x + echo_rect.width.saturating_sub(1)) as usize);
        return Some((x as u16, echo_rect.y));
    }

    let line = buf.line_of_point();
    let row = line as i64 - view.top_line as i64;
    if row < 0 || row >= body.height as i64 {
        return None;
    }
    let line_slice = visible_content(buf.line(line));
    let col_chars = buf.column().min(line_slice.len_chars());
    let vis_col = visual_col(line_slice.slice(..col_chars));
    let x = body.x + vis_col.min(body.width.saturating_sub(1) as usize) as u16;
    let y = body.y + row as u16;
    Some((x, y))
}

fn status_line(ed: &Editor) -> String {
    let buf = ed.buf();
    let mb = buf.rope().len_bytes() as f64 / 1e6;
    let modified = if buf.modified() { "**" } else { "--" };
    let ro = if buf.read_only() { "%" } else { "" };
    let file = buf
        .path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| buf.name().to_string());
    format!(
        " {}{}{}  L{} C{}  {} lines {:.1}MB",
        modified,
        ro,
        file,
        buf.line_of_point() + 1,
        buf.column(),
        buf.len_lines(),
        mb
    )
}
