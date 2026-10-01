//! Terminal rendering: window tree, modeline, echo area / minibuffer,
//! cursor. Long lines wrap to the window width (word-wrap), computed by
//! `emacs_core::wrap` so scrolling and rendering agree.

use emacs_core::editor::Editor;
use emacs_core::view::View;
use emacs_core::wrap::{self, RowWalker};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as TuiLine, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

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

fn expand_tabs(s: ropey::RopeSlice<'_>) -> String {
    let mut col = 0usize;
    let mut out = String::with_capacity(s.len_chars());
    for c in s.chars() {
        if c == '\t' {
            let spaces = wrap::TAB_WIDTH - col % wrap::TAB_WIDTH;
            out.extend(std::iter::repeat_n(' ', spaces));
            col += spaces;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

fn render_window(
    frame: &mut Frame,
    buf: &emacs_core::buffer::Buffer,
    view: &View,
    rect: Rect,
    search_match: Option<(usize, usize)>,
) {
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
    let width = text_rect.width.max(1) as usize;
    let region = buf.region();
    // Use the previous frame's hint to skip to the first visible row in
    // O(delta); validate it against the buffer length (edits invalidate).
    let hint = view.hint.get();
    let mut walker = if hint.3 == buf.len_chars() && hint.1 < buf.len_lines() {
        RowWalker::from_hint(buf, width, hint.0, hint.1, hint.2, view.top_row)
    } else {
        RowWalker::new(buf, width, view.top_row)
    };
    let mut lines: Vec<TuiLine> = Vec::new();
    let mut nums: Vec<TuiLine> = Vec::new();
    let mut first: Option<(usize, usize)> = None;
    for _ in 0..text_rect.height as usize {
        let Some((line_idx, seg, s, e)) = walker.next_row() else {
            break;
        };
        if first.is_none() {
            first = Some((line_idx, seg));
        }
        if line_numbers {
            // line numbers only on the first visual row of a wrapped line
            if seg == 0 {
                nums.push(TuiLine::styled(
                    format!("{:>width$} ", line_idx + 1, width = gutter_w as usize - 1),
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                nums.push(TuiLine::from(""));
            }
        }
        lines.push(render_segment(buf, line_idx, s, e, region, search_match));
    }
    if let Some((line, seg)) = first {
        view.hint
            .set((view.top_row, line, seg, buf.len_chars(), width));
    }
    if line_numbers && gutter_w > 0 && rect.width > gutter_w {
        let gutter = Rect {
            width: gutter_w,
            ..rect
        };
        frame.render_widget(Paragraph::new(nums), gutter);
    }
    frame.render_widget(Paragraph::new(lines), text_rect);
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

/// One wrapped segment of a line, rendered plain (tab-expanded). When the
/// mark is set, the columns between point and mark get the active-region
/// background (transient-mark-mode); the current isearch match gets a
/// yellow background. Highlight columns are mapped through tab expansion.
fn render_segment(
    buf: &emacs_core::buffer::Buffer,
    line_idx: usize,
    seg_start: usize,
    seg_end: usize,
    region: Option<(usize, usize)>,
    search_match: Option<(usize, usize)>,
) -> TuiLine<'static> {
    let line = buf.line(line_idx);
    let content = visible_content(line.slice(seg_start..seg_end));
    if content.len_chars() == 0 {
        return TuiLine::from("");
    }
    let plain = expand_tabs(content);
    let line_start = buf.rope().line_to_char(line_idx);
    let abs_start = line_start + seg_start;
    let abs_end = line_start + seg_end;
    let mut spans: Vec<Span<'static>> = vec![Span::raw(plain.clone())];
    if let Some((rs, re)) = region {
        if let Some((a, b)) = line_range(rs, re, abs_start, abs_end) {
            let (va, vb) = visual_range(content, a, b);
            spans = highlight_range(spans, &plain, va, vb, Color::LightBlue);
        }
    }
    if let Some((ss, se)) = search_match {
        if let Some((a, b)) = line_range(ss, se, abs_start, abs_end) {
            let (va, vb) = visual_range(content, a, b);
            spans = highlight_range(spans, &plain, va, vb, Color::Yellow);
        }
    }
    TuiLine::from(spans)
}

/// Intersection of a buffer char range with a segment's absolute range,
/// as segment-relative char columns.
fn line_range(
    start: usize,
    end: usize,
    seg_abs_start: usize,
    seg_abs_end: usize,
) -> Option<(usize, usize)> {
    let a = start.max(seg_abs_start).saturating_sub(seg_abs_start);
    let b = end.min(seg_abs_end).saturating_sub(seg_abs_start);
    (b > a).then_some((a, b))
}

/// Segment-relative char columns mapped to visual columns (tab-aware).
/// Clamped to the visible content (the trailing newline is stripped).
fn visual_range(content: ropey::RopeSlice<'_>, start: usize, end: usize) -> (usize, usize) {
    let len = content.len_chars();
    (
        wrap::visual_width(content.slice(..start.min(len))),
        wrap::visual_width(content.slice(..end.min(len))),
    )
}

/// Give the columns of `range` in `plain` the given background, splitting
/// the existing spans as needed.
fn highlight_range(
    spans: Vec<Span<'static>>,
    plain: &str,
    range_start: usize,
    range_end: usize,
    color: Color,
) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;
    for span in spans {
        let len = span.content.chars().count();
        let (s, e) = (col, col + len);
        let rs = range_start.max(s);
        let re = range_end.min(e);
        if re > rs {
            let before = slice_cols(&span.content, 0, rs - s);
            let inside = slice_cols(&span.content, rs - s, re - s);
            let after = slice_cols(&span.content, re - s, len);
            if !before.is_empty() {
                out.push(Span::styled(before.to_string(), span.style));
            }
            if !inside.is_empty() {
                out.push(Span::styled(inside.to_string(), span.style.bg(color)));
            }
            if !after.is_empty() {
                out.push(Span::styled(after.to_string(), span.style));
            }
        } else {
            out.push(span);
        }
        col = e;
    }
    let _ = plain;
    out
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

    let completing = ed
        .minibuffer()
        .is_some_and(|mb| mb.completion && !mb.candidates.is_empty() && mb.candidates.len() >= 2);
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
        let search_match = if l.selected { ed.search_match() } else { None };
        render_window(frame, l.buf, l.view, rect, search_match);
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
    let line_rect = if ed.minibuffer().is_some() && completing {
        minibuf_rect
    } else {
        echo_rect
    };

    // The minibuffer scrolls horizontally so the caret stays visible when
    // the line overflows the echo area (long prompts, long typed paths).
    // Space is reserved for the completion preview, which renders after
    // the caret.
    let mut scroll = 0usize;
    let echo_text: String = if let Some(mb) = ed.minibuffer() {
        let caret = if mb.buffer().point() == mb.buffer().len_chars() {
            "█"
        } else {
            ""
        };
        // caret sits between the typed input and the completion preview
        let line = format!("{}{}{}{}", mb.prompt, mb.input(), caret, mb.preview);
        let width = line_rect.width.max(1) as usize;
        let cursor_col = mb.prompt.chars().count() + mb.cursor_col();
        let preview_chars = mb.preview.chars().count();
        let keep = width.saturating_sub(1 + preview_chars);
        if cursor_col > keep {
            scroll = cursor_col - keep;
        }
        if scroll > 0 {
            line.chars().skip(scroll).collect()
        } else {
            line
        }
    } else if let Some(emacs_core::script::PendingRequest::ReadYesNo { prompt, .. }) = ed.pending()
    {
        prompt.clone()
    } else if let Some(msg) = ed.echo() {
        msg.to_string()
    } else {
        String::new()
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
        let x = (line_rect.x as usize + mb.prompt.chars().count() + mb.cursor_col())
            .saturating_sub(scroll)
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
    // find the cursor's visual row by walking the same wrapped rows that
    // were rendered (reusing the view's hint, so this is O(delta), not a
    // full walk from the top of the file every frame)
    let width = rect.width.max(1) as usize;
    let point = buf.point();
    let point_line = buf.line_of_point();
    let point_col = point - buf.rope().line_to_char(point_line);
    let hint = selected.view.hint.get();
    let mut walker = if hint.3 == buf.len_chars() && hint.1 < buf.len_lines() {
        RowWalker::from_hint(buf, width, hint.0, hint.1, hint.2, selected.view.top_row)
    } else {
        RowWalker::new(buf, width, selected.view.top_row)
    };
    for i in 0..rect.height as usize {
        let Some((line_idx, _seg, s, e)) = walker.next_row() else {
            break;
        };
        if line_idx == point_line && point_col >= s && point_col <= e {
            let vis = wrap::visual_width(buf.line(line_idx).slice(s..point_col));
            let gutter = if buf.minor_mode_enabled("line-numbers") {
                GUTTER_WIDTH
            } else {
                0
            };
            let x = rect.x + gutter + vis.min(rect.width.saturating_sub(1) as usize) as u16;
            let y = rect.y + i as u16;
            return Some((x, y));
        }
    }
    None
}
