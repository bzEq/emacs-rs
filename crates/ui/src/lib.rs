//! Terminal rendering: window tree, modeline, echo area / minibuffer, cursor.

use emacs_core::editor::Editor;
use emacs_core::syntax::{line_segments, Group};
use emacs_core::view::View;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as TuiLine, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub const TAB_WIDTH: usize = 8;
pub const GUTTER_WIDTH: u16 = 5;

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

fn render_window(frame: &mut Frame, buf: &emacs_core::buffer::Buffer, view: &View, rect: Rect) {
    let line_numbers = buf.minor_mode_enabled("line-numbers");
    let gutter_w = if line_numbers { GUTTER_WIDTH } else { 0 };
    let text_rect = if gutter_w > 0 && rect.width > gutter_w {
        Rect {
            x: rect.x + gutter_w,
            width: rect.width - gutter_w,
            ..rect
        }
    } else {
        rect
    };
    if line_numbers && gutter_w > 0 && rect.width > gutter_w {
        let gutter = Rect {
            width: gutter_w,
            ..rect
        };
        let nums: Vec<TuiLine> = (0..rect.height as usize)
            .filter_map(|i| {
                let line_idx = view.top_line + i;
                (line_idx < buf.len_lines()).then(|| {
                    TuiLine::styled(
                        format!("{:>width$} ", line_idx + 1, width = gutter_w as usize - 1),
                        Style::default().fg(Color::DarkGray),
                    )
                })
            })
            .collect();
        frame.render_widget(Paragraph::new(nums), gutter);
    }
    let lines: Vec<TuiLine> = (0..text_rect.height as usize)
        .filter_map(|i| {
            let line_idx = view.top_line + i;
            (line_idx < buf.len_lines()).then(|| render_line(buf, line_idx))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), text_rect);
}

fn style_for(group: Group) -> Style {
    let color = match group {
        Group::Keyword => Color::Magenta,
        Group::String => Color::Green,
        Group::Comment => Color::DarkGray,
        Group::Number => Color::Yellow,
        Group::Type => Color::Cyan,
        Group::Function => Color::Blue,
        Group::Constant => Color::Yellow,
    };
    Style::default().fg(color)
}

/// Slice by char columns.
fn slice_cols(s: &str, a: usize, b: usize) -> &str {
    let count = s.chars().count();
    let a = a.min(count);
    let b = b.min(count);
    if b <= a {
        return "";
    }
    let start = s.char_indices().nth(a).map(|(i, _)| i).unwrap_or(s.len());
    let end = s.char_indices().nth(b).map(|(i, _)| i).unwrap_or(s.len());
    &s[start..end]
}

/// One line with syntax highlighting. Lines containing tabs are rendered
/// plain (tab expansion would shift highlight columns).
fn render_line(buf: &emacs_core::buffer::Buffer, line_idx: usize) -> TuiLine<'static> {
    let content = visible_content(buf.line(line_idx));
    if content.len_chars() == 0 {
        return TuiLine::from("");
    }
    let plain = expand_tabs(content);
    let has_tab = content.chars().any(|c| c == '\t');
    let segs = buf
        .syntax()
        .map(|s| line_segments(s, buf, line_idx))
        .unwrap_or_default();
    if has_tab || segs.is_empty() {
        return TuiLine::from(plain);
    }
    let total = plain.chars().count();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut last = 0usize;
    for s in segs {
        let s_start = s.start.min(total);
        let s_end = s.end.min(total);
        if s_end <= s_start {
            continue;
        }
        if s_start > last {
            spans.push(Span::raw(slice_cols(&plain, last, s_start).to_string()));
        }
        spans.push(Span::styled(
            slice_cols(&plain, s_start, s_end).to_string(),
            style_for(s.group),
        ));
        last = s_end;
    }
    if last < total {
        spans.push(Span::raw(slice_cols(&plain, last, total).to_string()));
    }
    TuiLine::from(spans)
}

/// Modeline for a buffer, Emacs-style: `--`/`**` + `%` for read-only, name,
/// modes (major + enabled minors' lighters), point position, line count.
fn modeline(buf: &emacs_core::buffer::Buffer, ed: &Editor) -> String {
    let modified = if buf.modified() { "**" } else { "--" };
    let ro = if buf.read_only() { "%" } else { "-" };
    let file = buf
        .path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| buf.name().to_string());
    let mut modes = buf.mode().name.clone();
    for name in buf.enabled_minor() {
        if let Some(def) = ed.minor_def(name) {
            modes.push(' ');
            modes.push_str(&def.lighter);
        }
    }
    format!(
        "-{modified}{ro}-  {file}  ({modes})  L{} C{}  {} lines",
        buf.line_of_point() + 1,
        buf.column(),
        buf.len_lines()
    )
}

/// Render the editor: all windows, modeline, echo area (which doubles as the
/// minibuffer and grows to two lines while completion candidates are shown).
/// Returns the on-screen cursor position.
pub fn render(frame: &mut Frame, ed: &Editor) -> Option<(u16, u16)> {
    let area = frame.area();
    if area.height == 0 {
        return None;
    }

    let completing = ed.minibuffer().is_some_and(|mb| {
        mb.completion.is_some() && !mb.candidates.is_empty() && mb.candidates.len() >= 2
    });
    let echo_h: u16 = if completing { 2 } else { 1 };

    let body_h = area.height.saturating_sub(1 + echo_h);
    let modeline_rect = Rect {
        y: area.y + body_h,
        height: area.height - body_h - echo_h,
        ..area
    };
    let echo_rect = Rect {
        y: area.y + area.height - echo_h,
        height: echo_h,
        ..area
    };
    // the input line sits on the bottom row of the echo area
    let minibuf_rect = Rect {
        y: echo_rect.y + echo_h - 1,
        height: 1,
        ..echo_rect
    };

    // --- windows -----------------------------------------------------------
    let layouts = ed.window_layout();
    for l in &layouts {
        let rect = Rect {
            x: l.rect.x,
            y: l.rect.y,
            width: l.rect.w,
            height: l.rect.h,
        };
        render_window(frame, l.buf, l.view, rect);
    }

    // --- modeline ----------------------------------------------------------
    let ml_style = Style::default()
        .fg(Color::Black)
        .bg(Color::White)
        .add_modifier(Modifier::BOLD);
    if let Some(selected) = layouts.iter().find(|l| l.selected) {
        frame.render_widget(
            Paragraph::new(Span::styled(modeline(selected.buf, ed), ml_style)),
            modeline_rect,
        );
    }

    // --- echo area ---------------------------------------------------------
    let echo_style = if ed.echo_is_error() {
        Style::default().fg(Color::White).bg(Color::Red)
    } else {
        Style::default().fg(Color::Black).bg(Color::White)
    };
    if completing {
        // completion candidates on the top echo row
        let candidates: String = ed
            .minibuffer()
            .map(|mb| mb.candidates.join("  "))
            .unwrap_or_default();
        let cand_rect = Rect {
            height: 1,
            ..echo_rect
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                candidates,
                Style::default().fg(Color::DarkGray),
            )),
            cand_rect,
        );
    }
    let echo_text: String = if let Some(mb) = ed.minibuffer() {
        let caret = if mb.cursor == mb.input.chars().count() {
            "█"
        } else {
            ""
        };
        // caret sits between the typed input and the completion preview
        format!("{}{}{}{}", mb.prompt, mb.input, caret, mb.preview)
    } else if let Some(emacs_core::minibuffer::Pending::YesNo { prompt, .. }) = ed.pending() {
        prompt.clone()
    } else if let Some(msg) = ed.echo() {
        msg.to_string()
    } else {
        String::new()
    };
    let line_rect = if ed.minibuffer().is_some() && completing {
        minibuf_rect
    } else {
        echo_rect
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            echo_text,
            echo_style.add_modifier(Modifier::BOLD),
        )),
        line_rect,
    );

    // --- cursor ------------------------------------------------------------
    if let Some(mb) = ed.minibuffer() {
        let x = (line_rect.x as usize + mb.prompt.chars().count() + mb.cursor)
            .min((line_rect.x + line_rect.width.saturating_sub(1)) as usize);
        return Some((x as u16, line_rect.y));
    }

    let selected = layouts.iter().find(|l| l.selected)?;
    let buf = selected.buf;
    let rect = Rect {
        x: selected.rect.x,
        y: selected.rect.y,
        width: selected.rect.w,
        height: selected.rect.h,
    };
    if rect.height == 0 {
        return None;
    }
    let line = buf.line_of_point();
    let row = line as i64 - selected.view.top_line as i64;
    if row < 0 || row >= rect.height as i64 {
        return None;
    }
    let line_slice = visible_content(buf.line(line));
    let col_chars = buf.column().min(line_slice.len_chars());
    let vis_col = visual_col(line_slice.slice(..col_chars));
    let gutter = if buf.minor_mode_enabled("line-numbers") {
        GUTTER_WIDTH
    } else {
        0
    };
    let x = rect.x + gutter + vis_col.min(rect.width.saturating_sub(1) as usize) as u16;
    let y = rect.y + row as u16;
    Some((x, y))
}
