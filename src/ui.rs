use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{App, Focus};
use crate::buffer::Buffer;
use crate::highlight::UiPalette;
use crate::media::{self, BinaryDoc, Media};

const TREE_WIDTH: u16 = 32;

// A fixed accent that reads as intentional UI on any dark theme. Text, dim and
// selection are derived from the active theme instead (see UiPalette), so the
// chrome tracks the code; no background is ever painted, so the terminal's own
// background and any transparency show through, the status bar included.
const ACCENT: Color = Color::Rgb(97, 175, 239);

// Semantic status foregrounds: success, warning (unsaved / disk conflict) and
// error. Mid-tones chosen to stay legible on any dark background.
const OK: Color = Color::Rgb(152, 195, 121);
const WARN: Color = Color::Rgb(209, 154, 102);
const ERR: Color = Color::Rgb(224, 108, 117);

const IS_MAC: bool = cfg!(target_os = "macos");

/// A run of status-bar text with its own style. The bar is assembled from a
/// list of these so each part (mode badge, path, warning) is colored on its own.
type Seg = (String, Style);

/// ASCII wordmark shown on the empty-state welcome screen, centered as a block.
const LOGO: &[&str] = &[
    r"  ___  _ __   ___ _ __   ___ ___   __| | ___ ",
    r" / _ \| '_ \ / _ \ '_ \ / __/ _ \ / _ \|/ _ \",
    r"| (_) | |_) |  __/ | | | (_| (_) | (_| |  __/",
    r" \___/| .__/ \___|_| |_|\___\___/ \__,_|\___|",
    r"      |_|",
];

// Built with concat! so the leading indentation is part of each literal —
// a `\`-continuation would let Rust strip the spaces and flatten the preview.
const PREVIEW: &str = concat!(
    "// opencode — preview of this style\n",
    "use std::collections::HashMap;\n",
    "\n",
    "/// Greet a user by name.\n",
    "pub fn greet(name: &str, count: u32) -> String {\n",
    "    let mut seen: HashMap<&str, u32> = HashMap::new();\n",
    "    seen.insert(name, count);\n",
    "\n",
    "    if count > 0 {\n",
    "        format!(\"hi, {name}! x{count}\")\n",
    "    } else {\n",
    "        String::from(\"hello, world\")\n",
    "    }\n",
    "}\n",
);

pub fn render(frame: &mut Frame, app: &mut App) {
    let root = frame.area();

    if let Some(sel) = app.picker {
        let pal = app.highlighter.ui_palette_for(sel);

        render_picker(frame, app, root, pal);

        return;
    }

    let pal = app.highlighter.ui_palette();

    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(root);

    let main = chunks[0];

    let status_area = chunks[1];

    let editor_area = if app.tree_visible {
        let cols = Layout::horizontal([Constraint::Length(TREE_WIDTH), Constraint::Min(0)]).split(main);

        render_tree(frame, app, cols[0], pal);

        cols[1]
    } else {
        app.tree_area = None;

        main
    };

    render_editor(frame, app, editor_area, pal);

    render_status(frame, app, status_area, pal);
}

fn render_picker(frame: &mut Frame, app: &App, area: Rect, pal: UiPalette) {
    let sel = app.picker.unwrap_or(0);

    let count = app.highlighter.theme_count();

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .split(area);

    let title = Line::from(Span::styled(
        format!("  opencode — choose a style ({count} available)"),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ));

    frame.render_widget(Paragraph::new(title), rows[0]);

    let cols = Layout::horizontal([Constraint::Length(30), Constraint::Min(0)]).split(rows[1]);

    render_theme_list(frame, app, cols[0], sel, count, pal);

    render_preview(frame, app, cols[1], sel, pal);

    render_picker_footer(frame, rows[2], count, pal);
}

fn render_theme_list(frame: &mut Frame, app: &App, area: Rect, sel: usize, count: usize, pal: UiPalette) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(format!(" Styles ({count}) "));

    let inner = block.inner(area);

    frame.render_widget(block, area);

    let visible = inner.height as usize;

    if visible == 0 {
        return;
    }

    let offset = if sel >= visible { sel + 1 - visible } else { 0 };

    let names = app.highlighter.theme_names();

    let end = (offset + visible).min(names.len());

    let lines: Vec<Line> = names[offset..end]
        .iter()
        .enumerate()
        .map(|(row, name)| {
            let i = offset + row;

            let marker = if i == sel { "› " } else { "  " };

            let mut style = Style::default().fg(pal.fg);

            if i == sel {
                style = style.bg(pal.selection).add_modifier(Modifier::BOLD);
            }

            Line::from(Span::styled(format!("{marker}{name}"), style))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_preview(frame: &mut Frame, app: &App, area: Rect, sel: usize, pal: UiPalette) {
    let names = app.highlighter.theme_names();

    let tag = if app.highlighter.theme_is_dark(sel) { "dark" } else { "light" };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(format!(" Preview · {} · {tag} ", names.get(sel).copied().unwrap_or("")));

    let inner = block.inner(area);

    frame.render_widget(block, area);

    let blocks = app.highlighter.highlight_block(PREVIEW, "rs", sel);

    let num_w = blocks.len().to_string().len().max(2);

    let lines: Vec<Line> = blocks
        .iter()
        .enumerate()
        .map(|(i, spans)| {
            let mut out = vec![Span::styled(
                format!("{:>num_w$} ", i + 1),
                Style::default().fg(pal.dim),
            )];

            out.extend(spans.iter().map(|(style, text)| Span::styled(text.clone(), *style)));

            Line::from(out)
        })
        .collect();

    // No background: only the syntax foreground colors are drawn, so the user's
    // own terminal background is preserved exactly as in the editor.
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_picker_footer(frame: &mut Frame, area: Rect, count: usize, pal: UiPalette) {
    let word_jump = if IS_MAC { "⌥+←/→" } else { "Ctrl+←/→" };

    let keys = Line::from(Span::styled(
        "  ↑/↓ select     Enter apply & save     Esc quit",
        Style::default().fg(pal.fg),
    ));

    let info = Line::from(Span::styled(
        format!(
            "  {count} styles · word-jump {word_jump} · add .tmTheme in {} · change later: ocode --style",
            crate::config::themes_dir_display()
        ),
        Style::default().fg(pal.dim),
    ));

    frame.render_widget(Paragraph::new(vec![keys, info]), area);
}

fn render_tree(frame: &mut Frame, app: &mut App, area: Rect, pal: UiPalette) {
    let title = app
        .tree
        .root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".")
        .to_string();

    let focused = app.focus == Focus::Tree;

    let border_style = if focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(pal.dim)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(format!(" {title} "));

    let inner = block.inner(area);

    frame.render_widget(block, area);

    app.tree_area = Some((inner.x, inner.y, inner.width, inner.height));

    let visible = inner.height as usize;

    if visible == 0 {
        return;
    }

    // Never leave the window past the end (the list shrinks when a directory
    // collapses or files disappear).
    app.tree.scroll = app.tree.scroll.min(app.tree.nodes.len().saturating_sub(visible));

    // While the wheel is driving the list, stop chasing the selection.
    if !app.tree_scroll_free {
        if app.tree.selected < app.tree.scroll {
            app.tree.scroll = app.tree.selected;
        } else if app.tree.selected >= app.tree.scroll + visible {
            app.tree.scroll = app.tree.selected + 1 - visible;
        }
    }

    let mut lines: Vec<Line> = Vec::with_capacity(visible);

    let end = (app.tree.scroll + visible).min(app.tree.nodes.len());

    for idx in app.tree.scroll..end {
        let node = &app.tree.nodes[idx];

        let indent = "  ".repeat(node.depth);

        let marker = if node.is_dir {
            if node.expanded {
                "▾ "
            } else {
                "▸ "
            }
        } else {
            "  "
        };

        let fg = if node.is_dir { ACCENT } else { pal.fg };

        let mut style = Style::default().fg(fg);

        if idx == app.tree.selected {
            style = style.bg(pal.selection).add_modifier(Modifier::BOLD);
        }

        let text = format!("{indent}{marker}{}", node.name);

        lines.push(Line::from(Span::styled(text, style)));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Centered logo + tagline shown when no file is open yet.
fn render_welcome(frame: &mut Frame, area: Rect, pal: UiPalette) {
    let width = area.width as usize;

    let logo_w = LOGO.iter().map(|l| l.chars().count()).max().unwrap_or(0);

    let logo_pad = " ".repeat(width.saturating_sub(logo_w) / 2);

    let nav = if IS_MAC { "⌥/Shift+arrows" } else { "Ctrl/Shift+arrows" };

    let mut content: Vec<Line> = LOGO
        .iter()
        .map(|l| {
            Line::from(Span::styled(
                format!("{logo_pad}{l}"),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))
        })
        .collect();

    content.push(Line::from(""));

    content.push(centered(width, "a fast terminal code reader & editor", Style::default().fg(pal.fg)));

    content.push(Line::from(""));

    content.push(centered(width, "Enter or Ctrl+B  browse files        Ctrl+Q  quit", Style::default().fg(pal.dim)));

    content.push(centered(width, &format!("move with {nav}  ·  Ctrl+S save  ·  Ctrl+Z undo"), Style::default().fg(pal.dim)));

    let top = (area.height as usize).saturating_sub(content.len()) / 2;

    let mut lines: Vec<Line> = vec![Line::from(""); top];

    lines.extend(content);

    frame.render_widget(Paragraph::new(lines), area);
}

fn centered(width: usize, text: &str, style: Style) -> Line<'static> {
    let pad = " ".repeat(width.saturating_sub(text.chars().count()) / 2);

    Line::from(Span::styled(format!("{pad}{text}"), style))
}

fn render_editor(frame: &mut Frame, app: &mut App, area: Rect, pal: UiPalette) {
    if app.media.is_some() {
        app.editor_area = None;

        render_media(frame, app, area, pal);

        return;
    }

    if app.buffer.is_none() {
        app.editor_area = None;

        render_welcome(frame, area, pal);

        return;
    }

    let Some(buf) = app.buffer.as_mut() else {
        return;
    };

    let height = area.height as usize;

    let width = area.width as usize;

    if height == 0 || width == 0 {
        return;
    }

    app.page_rows = height;

    let total = buf.last_line() + 1;

    let num_w = total.to_string().len().max(3);

    let gutter_w = num_w + 1;

    let text_width = width.saturating_sub(gutter_w);

    app.editor_area = Some((area.x, area.y, area.width, area.height));

    app.gutter_w = gutter_w as u16;

    // While the wheel has scrolled away from the caret the view stays put; any
    // keystroke clears the flag and the caret pulls it back.
    if !app.scroll_free {
        scroll_into_view(buf, height, text_width);
    }

    let target = buf.scroll_row + height;

    app.highlighter.ensure(&mut buf.hl, &buf.rope, target);

    let mut lines: Vec<Line> = Vec::with_capacity(height);

    let last = buf.last_line();

    for row in 0..height {
        let li = buf.scroll_row + row;

        if li > last {
            break;
        }

        let num_style = if li == buf.cursor_line {
            Style::default().fg(pal.fg)
        } else {
            Style::default().fg(pal.dim)
        };

        let gutter = Span::styled(format!("{:>w$} ", li + 1, w = num_w), num_style);

        let mut spans = vec![gutter];

        let sel = buf.selection_for_line(li);

        if let Some(cached) = buf.hl.line(li) {
            spans.extend(slice_spans(cached, buf.scroll_col, text_width, sel, pal.selection));
        }

        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), area);

    // A wheel scroll can leave the caret off screen; placing it then would
    // underflow, so the terminal cursor is simply hidden until it is back.
    let caret_visible = buf.cursor_line >= buf.scroll_row
        && buf.cursor_line < buf.scroll_row + height
        && buf.cursor_col >= buf.scroll_col
        && buf.cursor_col - buf.scroll_col < text_width.max(1);

    if app.focus == Focus::Editor && app.search.is_none() && caret_visible {
        let cx = area.x + gutter_w as u16 + (buf.cursor_col - buf.scroll_col) as u16;

        let cy = area.y + (buf.cursor_line - buf.scroll_row) as u16;

        frame.set_cursor_position((cx, cy));
    }
}

fn render_media(frame: &mut Frame, app: &mut App, area: Rect, pal: UiPalette) {
    if matches!(app.media, Some(Media::Image(_))) {
        // Leave the area blank; the run loop paints the image into it. This is
        // the editor pane, so it already sits clear of the sidebar and the
        // image is simply scaled into whatever width is left.
        app.image_cells = (area.width > 0 && area.height > 0)
            .then_some((area.x, area.y, area.width, area.height));

        return;
    }

    app.image_cells = None;

    if let Some(Media::Binary(doc)) = &app.media {
        render_binary_info(frame, doc, area, pal);
    }
}

fn render_binary_info(frame: &mut Frame, doc: &BinaryDoc, area: Rect, pal: UiPalette) {
    let name = doc.path.file_name().and_then(|n| n.to_str()).unwrap_or("file");

    let mut lines: Vec<Line> = vec![Line::from("")];

    lines.push(Line::from(Span::styled(
        format!("  {name}"),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )));

    lines.push(Line::from(Span::styled(
        format!("  {} · {}", doc.format, media::human_size(doc.byte_len)),
        Style::default().fg(pal.fg),
    )));

    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        "  Not text — hex preview of the first bytes:",
        Style::default().fg(pal.dim),
    )));

    lines.push(Line::from(""));

    for chunk in doc.head.chunks(16) {
        lines.push(Line::from(Span::styled(hex_line(chunk), Style::default().fg(pal.fg))));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn hex_line(chunk: &[u8]) -> String {
    let mut hex = String::new();

    for (i, b) in chunk.iter().enumerate() {
        if i == 8 {
            hex.push(' ');
        }

        hex.push_str(&format!("{b:02x} "));
    }

    let ascii: String = chunk
        .iter()
        .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' })
        .collect();

    format!("  {hex:<49}|{ascii}|")
}

fn render_status(frame: &mut Frame, app: &App, area: Rect, pal: UiPalette) {
    let width = area.width as usize;

    let badge = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);

    let text = Style::default().fg(pal.fg);

    let dim = Style::default().fg(pal.dim);

    let warn = Style::default().fg(WARN).add_modifier(Modifier::BOLD);

    let conflict = app.search.is_none() && app.buffer.as_ref().is_some_and(|b| b.disk_changed);

    let (left, right): (Vec<Seg>, Vec<Seg>) = if let Some(search) = &app.search {
        (
            vec![(" Find: ".to_string(), badge), (search.query.clone(), text)],
            vec![(" Enter: next  Esc: close ".to_string(), dim)],
        )
    } else if conflict {
        let buf = app.buffer.as_ref().unwrap();

        (
            vec![(
                format!(" ⚠ {} changed on disk — Ctrl+R reload · Ctrl+S overwrite", buf.file_name()),
                warn,
            )],
            vec![(format!("Ln {}, Col {} ", buf.cursor_line + 1, buf.cursor_col + 1), warn)],
        )
    } else if let Some(buf) = &app.buffer {
        let focus = if app.focus == Focus::Tree { "TREE" } else { "EDIT" };

        let mut left = vec![
            (format!(" [{focus}] "), badge),
            (buf.path.display().to_string(), text),
        ];

        if buf.modified {
            left.push(("*".to_string(), warn));
        }

        if !app.status.is_empty() {
            left.push((format!("  {}", app.status), flash_style(app.status_ok)));
        }

        (
            left,
            vec![(format!("Ln {}, Col {} ", buf.cursor_line + 1, buf.cursor_col + 1), dim)],
        )
    } else if let Some(m) = &app.media {
        let (path, info) = match m {
            Media::Image(d) => (
                d.path.display().to_string(),
                format!("{} · {}×{} · {}", d.format, d.width, d.height, media::human_size(d.byte_len)),
            ),

            Media::Binary(d) => (
                d.path.display().to_string(),
                format!("{} · {}", d.format, media::human_size(d.byte_len)),
            ),
        };

        (
            vec![(" [VIEW] ".to_string(), badge), (path, text)],
            vec![(format!("{info} "), dim)],
        )
    } else {
        let mut left = vec![(" [TREE] ".to_string(), badge)];

        if app.status.is_empty() {
            left.push(("opencode".to_string(), dim));
        } else {
            left.push((app.status.clone(), flash_style(app.status_ok)));
        }

        (left, Vec::new())
    };

    frame.render_widget(Paragraph::new(compose_bar(&left, &right, width)), area);
}

/// Green for a success flash, red for a lingering error (see `App::flash` /
/// `App::set_error`).
fn flash_style(ok: bool) -> Style {
    if ok {
        Style::default().fg(OK).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(ERR)
    }
}

/// Lay out a status bar exactly `width` columns wide from styled segments: pin
/// `right` to the right edge and truncate `left` from its head (keeping the
/// tail, e.g. the file name) when the two would not fit. No background is drawn,
/// so the terminal's own background and any transparency show through.
fn compose_bar(left: &[Seg], right: &[Seg], width: usize) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }

    let right_w = seg_len(right).min(width);

    let avail = width - right_w;

    let left_len = seg_len(left);

    let mut spans: Vec<Span<'static>> = Vec::new();

    if left_len <= avail {
        for (t, s) in left {
            spans.push(Span::styled(t.clone(), *s));
        }

        spans.push(Span::raw(" ".repeat(avail - left_len)));
    } else if avail > 0 {
        spans.push(Span::raw("…"));

        spans.extend(tail_spans(left, avail - 1));
    }

    spans.extend(tail_spans(right, right_w));

    Line::from(spans)
}

fn seg_len(segs: &[Seg]) -> usize {
    segs.iter().map(|(t, _)| t.chars().count()).sum()
}

/// The last `keep` characters of a styled segment list, preserving each
/// segment's style and splitting the boundary segment.
fn tail_spans(segs: &[Seg], keep: usize) -> Vec<Span<'static>> {
    if keep == 0 {
        return Vec::new();
    }

    let skip = seg_len(segs).saturating_sub(keep);

    let mut out: Vec<Span<'static>> = Vec::new();

    let mut idx = 0;

    for (text, style) in segs {
        let seg_start = idx;

        idx += text.chars().count();

        if idx <= skip {
            continue;
        }

        let start_in_seg = skip.saturating_sub(seg_start);

        let kept: String = text.chars().skip(start_in_seg).collect();

        if !kept.is_empty() {
            out.push(Span::styled(kept, *style));
        }
    }

    out
}

fn scroll_into_view(buf: &mut Buffer, height: usize, text_width: usize) {
    if buf.cursor_line < buf.scroll_row {
        buf.scroll_row = buf.cursor_line;
    } else if buf.cursor_line >= buf.scroll_row + height {
        buf.scroll_row = buf.cursor_line + 1 - height;
    }

    if text_width == 0 {
        return;
    }

    if buf.cursor_col < buf.scroll_col {
        buf.scroll_col = buf.cursor_col;
    } else if buf.cursor_col >= buf.scroll_col + text_width {
        buf.scroll_col = buf.cursor_col + 1 - text_width;
    }
}

/// Take the slice of highlighted spans visible in `[start, start + width)`
/// display columns, expanding tabs to a single space so columns stay aligned
/// with the cursor (which counts one column per char). Columns inside `sel`
/// (line-relative char range) get the selection background.
fn slice_spans(
    spans: &[(Style, String)],
    start: usize,
    width: usize,
    sel: Option<(usize, usize)>,
    sel_bg: Color,
) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();

    if width == 0 {
        return out;
    }

    let stop = start + width;

    let mut col = 0usize;

    let mut chunk = String::new();

    let mut chunk_style: Option<Style> = None;

    'outer: for (style, text) in spans {
        for ch in text.chars() {
            if col >= stop {
                break 'outer;
            }

            if col >= start {
                let selected = sel.is_some_and(|(a, b)| col >= a && col < b);

                let cell_style = if selected { style.bg(sel_bg) } else { *style };

                if chunk_style != Some(cell_style) {
                    flush_chunk(&mut out, &mut chunk, chunk_style);

                    chunk_style = Some(cell_style);
                }

                chunk.push(if ch == '\t' { ' ' } else { ch });
            }

            col += 1;
        }
    }

    flush_chunk(&mut out, &mut chunk, chunk_style);

    out
}

fn flush_chunk(out: &mut Vec<Span<'static>>, chunk: &mut String, style: Option<Style>) {
    if let Some(style) = style {
        if !chunk.is_empty() {
            out.push(Span::styled(std::mem::take(chunk), style));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    use crate::app::App;

    fn render_to_string(app: &mut App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();

        terminal.draw(|frame| super::render(frame, app)).unwrap();

        format!("{}", terminal.backend())
    }

    #[test]
    fn editor_renders_code_with_line_numbers() {
        let path = std::env::temp_dir().join("opencode_render_test.py");

        fs::write(&path, "def greet(name):\n    return name\n").unwrap();

        let mut app = App::new(path.clone(), false).unwrap();

        app.picker = None;

        let screen = render_to_string(&mut app, 80, 24);

        assert!(screen.contains("def greet"), "code not rendered:\n{screen}");

        assert!(screen.contains("return name"), "second line missing");

        assert!(screen.contains("Ln 1, Col 1"), "status line missing");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn status_bar_paints_no_opaque_background() {
        let path = std::env::temp_dir().join("opencode_status_bg.rs");

        fs::write(&path, "fn main() {}\n").unwrap();

        let mut app = App::new(path.clone(), false).unwrap();

        app.picker = None;

        let (w, h) = (80u16, 6u16);

        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();

        terminal.draw(|frame| super::render(frame, &mut app)).unwrap();

        let buffer = terminal.backend().buffer();

        // The status bar is the last row; every cell must keep the terminal's
        // own background (Reset) so transparency is preserved.
        for x in 0..w {
            let bg = buffer.cell((x, h - 1)).unwrap().bg;

            assert_eq!(bg, Color::Reset, "status cell at x={x} paints an opaque background {bg:?}");
        }

        let _ = fs::remove_file(path);
    }

    #[test]
    fn render_records_the_editor_area_for_mouse_mapping() {
        let path = std::env::temp_dir().join("opencode_mouse_area.rs");

        fs::write(&path, "fn main() {}\n").unwrap();

        let mut app = App::new(path.clone(), false).unwrap();

        app.picker = None;

        let _ = render_to_string(&mut app, 80, 6);

        let area = app.editor_area.expect("editor area recorded for the mouse");

        assert_eq!(area, (0, 0, 80, 5), "status bar takes the last row");

        assert_eq!(app.gutter_w, 4, "3-digit line numbers plus a space");

        let _ = fs::remove_file(path);
    }

    /// An image opened with the sidebar up must still be painted, in the editor
    /// pane beside the tree rather than suppressed.
    #[test]
    fn image_renders_beside_an_open_sidebar() {
        let dir = std::env::temp_dir().join("opencode_img_sidebar");

        let _ = fs::remove_dir_all(&dir);

        fs::create_dir_all(&dir).unwrap();

        let path = dir.join("p.png");

        let mut img = image::RgbaImage::new(20, 10);

        for px in img.pixels_mut() {
            *px = image::Rgba([10, 20, 30, 255]);
        }

        image::DynamicImage::ImageRgba8(img).save(&path).unwrap();

        let mut app = App::new(path, false).unwrap();

        app.picker = None;

        app.tree_visible = true;

        let _ = render_to_string(&mut app, 80, 10);

        let cells = app.image_cells.expect("image still painted with the tree open");

        assert_eq!(cells.0, 32, "placed to the right of the 32-column sidebar");

        assert_eq!(cells.2, 48, "and given the remaining width");

        let _ = fs::remove_dir_all(dir);
    }

    fn write_png(path: &std::path::Path, w: u32, h: u32) {
        let mut img = image::RgbaImage::new(w, h);

        for px in img.pixels_mut() {
            *px = image::Rgba([10, 20, 30, 255]);
        }

        image::DynamicImage::ImageRgba8(img).save(path).unwrap();
    }

    fn left_click(column: u16, row: u16) -> crossterm::event::MouseEvent {
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }
    }

    /// Two images opened one after the other land on the same pane box, so the
    /// placement has to differ by image or the run loop skips the repaint and
    /// leaves the first one on screen.
    #[test]
    fn switching_images_changes_the_placement() {
        let dir = std::env::temp_dir().join("opencode_img_switch");

        let _ = fs::remove_dir_all(&dir);

        fs::create_dir_all(&dir).unwrap();

        // Same size on purpose: identical cell box, so only the image differs.
        write_png(&dir.join("a.png"), 20, 10);

        write_png(&dir.join("b.png"), 20, 10);

        let mut app = App::new(dir.clone(), false).unwrap();

        app.picker = None;

        app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Enter));

        // Render once so the sidebar geometry clicks map against is real.
        let _ = render_to_string(&mut app, 80, 12);

        let (_, ty, _, _) = app.tree_area.expect("sidebar recorded");

        let row_of = |app: &App, name: &str| {
            let idx = app.tree.nodes.iter().position(|n| n.name == name).expect("row");

            ty + (idx - app.tree.scroll) as u16
        };

        // Two clicks to open: the first only selects.
        let ra = row_of(&app, "a.png");

        app.on_mouse(left_click(2, ra));

        app.on_mouse(left_click(2, ra));

        let _ = render_to_string(&mut app, 80, 12);

        let first = app.image_placement().expect("a.png placed");

        let rb = row_of(&app, "b.png");

        app.on_mouse(left_click(2, rb));

        app.on_mouse(left_click(2, rb));

        let _ = render_to_string(&mut app, 80, 12);

        let second = app.image_placement().expect("b.png placed");

        assert_eq!(first.1, second.1, "same geometry: the box alone cannot tell them apart");

        assert_ne!(first, second, "so the placement must differ by image");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn image_is_centered_in_the_pane() {
        let dir = std::env::temp_dir().join("opencode_img_center");

        let _ = fs::remove_dir_all(&dir);

        fs::create_dir_all(&dir).unwrap();

        let path = dir.join("wide.png");

        // 2:1 pixels means 4 cells wide per cell tall, so an 11-row pane fits
        // 44 columns of image inside 80, leaving 36 columns to split evenly.
        write_png(&path, 20, 10);

        let mut app = App::new(path, false).unwrap();

        app.picker = None;

        let _ = render_to_string(&mut app, 80, 12);

        let (_, (x, y, cols, rows)) = app.image_placement().expect("image placed");

        assert_eq!((cols, rows), (44, 11), "fitted to the pane, aspect preserved");

        assert_eq!(x, (80 - 44) / 2, "centered horizontally");

        assert_eq!(y, 0, "full height, so nothing to centre vertically");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn tree_renders_directory() {
        let dir = std::env::temp_dir().join("opencode_tree_test");

        let _ = fs::create_dir_all(&dir);

        fs::write(dir.join("readme.md"), "# hi").unwrap();

        let mut app = App::new(PathBuf::from(&dir), false).unwrap();

        app.picker = None;

        app.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Enter)); // open the browser

        let screen = render_to_string(&mut app, 80, 24);

        assert!(screen.contains("readme.md"), "tree entry missing:\n{screen}");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn welcome_screen_shows_logo_when_no_file() {
        let dir = std::env::temp_dir().join("opencode_welcome_test");

        let _ = std::fs::create_dir_all(&dir);

        std::fs::write(dir.join("x.txt"), "x").unwrap();

        let mut app = App::new(PathBuf::from(&dir), false).unwrap();

        app.picker = None;

        assert!(app.buffer.is_none(), "directory launch should open no file");

        let screen = render_to_string(&mut app, 100, 26);

        assert!(screen.contains("terminal code reader"), "welcome tagline missing:\n{screen}");

        assert!(!screen.contains("no file open"), "old empty-state text should be gone");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn preview_keeps_indentation() {
        let path = std::env::temp_dir().join("opencode_indent.rs");

        std::fs::write(&path, "x").unwrap();

        let mut app = App::new(path.clone(), true).unwrap();

        let screen = render_to_string(&mut app, 110, 26);

        // The body lines must keep their leading whitespace in the preview.
        assert!(
            screen.contains("    seen.insert"),
            "preview lost indentation:\n{screen}"
        );

        assert!(
            screen.contains("        format!"),
            "preview lost nested indentation"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn picker_lists_styles_with_preview() {
        let path = std::env::temp_dir().join("opencode_picker_test.rs");

        fs::write(&path, "fn x() {}\n").unwrap();

        let mut app = App::new(path.clone(), true).unwrap();

        assert!(app.picker.is_some(), "picker should show when forced");

        let screen = render_to_string(&mut app, 110, 30);

        assert!(screen.contains("choose a style"), "title missing:\n{screen}");

        assert!(screen.contains("Dracula"), "bundled theme missing from list");

        assert!(screen.contains("Preview"), "preview pane missing");

        assert!(screen.contains("greet"), "preview snippet missing");

        let _ = fs::remove_file(path);
    }
}
