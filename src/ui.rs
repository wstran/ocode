use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{App, Focus};
use crate::buffer::Buffer;
use crate::media::{self, BinaryDoc, Media};

const TREE_WIDTH: u16 = 32;
const STATUS_BG: Color = Color::Rgb(40, 44, 52);
const GUTTER_FG: Color = Color::Rgb(92, 99, 112);
const GUTTER_CUR: Color = Color::Rgb(171, 178, 191);
const SEL_BG: Color = Color::Rgb(55, 62, 76);
const SEL_HL: Color = Color::Rgb(54, 78, 120);
const ACCENT: Color = Color::Rgb(97, 175, 239);
const TEXT_FG: Color = Color::Rgb(171, 178, 191);

const IS_MAC: bool = cfg!(target_os = "macos");

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

    if app.picker.is_some() {
        render_picker(frame, app, root);

        return;
    }

    let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(root);

    let main = chunks[0];

    let status_area = chunks[1];

    let editor_area = if app.tree_visible {
        let cols = Layout::horizontal([Constraint::Length(TREE_WIDTH), Constraint::Min(0)]).split(main);

        render_tree(frame, app, cols[0]);

        cols[1]
    } else {
        main
    };

    render_editor(frame, app, editor_area);

    render_status(frame, app, status_area);
}

fn render_picker(frame: &mut Frame, app: &App, area: Rect) {
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

    render_theme_list(frame, app, cols[0], sel, count);

    render_preview(frame, app, cols[1], sel);

    render_picker_footer(frame, rows[2], count);
}

fn render_theme_list(frame: &mut Frame, app: &App, area: Rect, sel: usize, count: usize) {
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

            let mut style = Style::default().fg(TEXT_FG);

            if i == sel {
                style = style.bg(SEL_BG).add_modifier(Modifier::BOLD);
            }

            Line::from(Span::styled(format!("{marker}{name}"), style))
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_preview(frame: &mut Frame, app: &App, area: Rect, sel: usize) {
    let names = app.highlighter.theme_names();

    let tag = if app.highlighter.theme_is_dark(sel) { "dark" } else { "light" };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GUTTER_FG))
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
                Style::default().fg(GUTTER_FG),
            )];

            out.extend(spans.iter().map(|(style, text)| Span::styled(text.clone(), *style)));

            Line::from(out)
        })
        .collect();

    // No background: only the syntax foreground colors are drawn, so the user's
    // own terminal background is preserved exactly as in the editor.
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_picker_footer(frame: &mut Frame, area: Rect, count: usize) {
    let word_jump = if IS_MAC { "⌥+←/→" } else { "Ctrl+←/→" };

    let keys = Line::from(Span::styled(
        "  ↑/↓ select     Enter apply & save     Esc quit",
        Style::default().fg(TEXT_FG),
    ));

    let info = Line::from(Span::styled(
        format!(
            "  {count} styles · word-jump {word_jump} · add .tmTheme in {} · change later: ocode --style",
            crate::config::themes_dir_display()
        ),
        Style::default().fg(GUTTER_FG),
    ));

    frame.render_widget(Paragraph::new(vec![keys, info]), area);
}

fn render_tree(frame: &mut Frame, app: &mut App, area: Rect) {
    let title = app
        .tree
        .root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".")
        .to_string();

    let focused = app.focus == Focus::Tree;

    let border_style = if focused {
        Style::default().fg(Color::Rgb(97, 175, 239))
    } else {
        Style::default().fg(GUTTER_FG)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(format!(" {title} "));

    let inner = block.inner(area);

    frame.render_widget(block, area);

    let visible = inner.height as usize;

    if visible == 0 {
        return;
    }

    if app.tree.selected < app.tree.scroll {
        app.tree.scroll = app.tree.selected;
    } else if app.tree.selected >= app.tree.scroll + visible {
        app.tree.scroll = app.tree.selected + 1 - visible;
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

        let fg = if node.is_dir {
            Color::Rgb(97, 175, 239)
        } else {
            Color::Rgb(171, 178, 191)
        };

        let mut style = Style::default().fg(fg);

        if idx == app.tree.selected {
            style = style.bg(SEL_BG).add_modifier(Modifier::BOLD);
        }

        let text = format!("{indent}{marker}{}", node.name);

        lines.push(Line::from(Span::styled(text, style)));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Centered logo + tagline shown when no file is open yet.
fn render_welcome(frame: &mut Frame, area: Rect) {
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

    content.push(centered(width, "a fast terminal code reader & editor", Style::default().fg(TEXT_FG)));

    content.push(Line::from(""));

    content.push(centered(width, "Ctrl+B  open a file        Ctrl+Q  quit", Style::default().fg(GUTTER_FG)));

    content.push(centered(width, &format!("move with {nav}  ·  Ctrl+S save  ·  Ctrl+Z undo"), Style::default().fg(GUTTER_FG)));

    let top = (area.height as usize).saturating_sub(content.len()) / 2;

    let mut lines: Vec<Line> = vec![Line::from(""); top];

    lines.extend(content);

    frame.render_widget(Paragraph::new(lines), area);
}

fn centered(width: usize, text: &str, style: Style) -> Line<'static> {
    let pad = " ".repeat(width.saturating_sub(text.chars().count()) / 2);

    Line::from(Span::styled(format!("{pad}{text}"), style))
}

fn render_editor(frame: &mut Frame, app: &mut App, area: Rect) {
    if app.media.is_some() {
        render_media(frame, app, area);

        return;
    }

    let Some(buf) = app.buffer.as_mut() else {
        render_welcome(frame, area);

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

    scroll_into_view(buf, height, text_width);

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
            Style::default().fg(GUTTER_CUR)
        } else {
            Style::default().fg(GUTTER_FG)
        };

        let gutter = Span::styled(format!("{:>w$} ", li + 1, w = num_w), num_style);

        let mut spans = vec![gutter];

        let sel = buf.selection_for_line(li);

        if let Some(cached) = buf.hl.line(li) {
            spans.extend(slice_spans(cached, buf.scroll_col, text_width, sel));
        }

        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), area);

    if app.focus == Focus::Editor && app.search.is_none() {
        let cx = area.x + gutter_w as u16 + (buf.cursor_col - buf.scroll_col) as u16;

        let cy = area.y + (buf.cursor_line - buf.scroll_row) as u16;

        frame.set_cursor_position((cx, cy));
    }
}

fn render_media(frame: &mut Frame, app: &mut App, area: Rect) {
    if matches!(app.media, Some(Media::Image(_))) {
        // The tree would overlap a kitty image, so hide the image while it is
        // open and tell the user how to view it.
        if app.tree_visible {
            app.image_cells = None;

            let line = Line::from(Span::styled(
                "  image — press Ctrl+B to hide the tree and view it",
                Style::default().fg(GUTTER_FG),
            ));

            frame.render_widget(Paragraph::new(line), area);
        } else {
            // Leave the area blank; the run loop paints the image into it.
            app.image_cells = (area.width > 0 && area.height > 0)
                .then_some((area.x, area.y, area.width, area.height));
        }

        return;
    }

    app.image_cells = None;

    if let Some(Media::Binary(doc)) = &app.media {
        render_binary_info(frame, doc, area);
    }
}

fn render_binary_info(frame: &mut Frame, doc: &BinaryDoc, area: Rect) {
    let name = doc.path.file_name().and_then(|n| n.to_str()).unwrap_or("file");

    let mut lines: Vec<Line> = vec![Line::from("")];

    lines.push(Line::from(Span::styled(
        format!("  {name}"),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    )));

    lines.push(Line::from(Span::styled(
        format!("  {} · {}", doc.format, media::human_size(doc.byte_len)),
        Style::default().fg(TEXT_FG),
    )));

    lines.push(Line::from(""));

    lines.push(Line::from(Span::styled(
        "  Not text — hex preview of the first bytes:",
        Style::default().fg(GUTTER_FG),
    )));

    lines.push(Line::from(""));

    for chunk in doc.head.chunks(16) {
        lines.push(Line::from(Span::styled(hex_line(chunk), Style::default().fg(TEXT_FG))));
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

fn render_status(frame: &mut Frame, app: &App, area: Rect) {
    let conflict = app.search.is_none() && app.buffer.as_ref().is_some_and(|b| b.disk_changed);

    let base = if conflict {
        Style::default()
            .bg(Color::Rgb(176, 132, 47))
            .fg(Color::Rgb(24, 24, 24))
            .add_modifier(Modifier::BOLD)
    } else if app.status_ok {
        Style::default()
            .bg(Color::Rgb(63, 115, 74))
            .fg(Color::Rgb(228, 240, 228))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(STATUS_BG).fg(TEXT_FG)
    };

    let width = area.width as usize;

    let (left, right) = if let Some(search) = &app.search {
        (format!(" Find: {}", search.query), " Enter: next  Esc: close ".to_string())
    } else if conflict {
        let buf = app.buffer.as_ref().unwrap();

        let right = format!("Ln {}, Col {} ", buf.cursor_line + 1, buf.cursor_col + 1);

        (
            format!(" ⚠ {} changed on disk — Ctrl+R reload · Ctrl+S overwrite", buf.file_name()),
            right,
        )
    } else if let Some(buf) = &app.buffer {
        let dirty = if buf.modified { "*" } else { "" };

        let focus = if app.focus == Focus::Tree { "TREE" } else { "EDIT" };

        let status = if app.status.is_empty() {
            String::new()
        } else {
            format!("  {}", app.status)
        };

        let left = format!(" [{focus}] {}{dirty}{status}", buf.path.display());

        let right = format!("Ln {}, Col {} ", buf.cursor_line + 1, buf.cursor_col + 1);

        (left, right)
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

        (format!(" [VIEW] {path}"), format!("{info} "))
    } else {
        let status = if app.status.is_empty() { "opencode" } else { &app.status };

        (format!(" [TREE] {status}"), String::new())
    };

    frame.render_widget(Paragraph::new(compose_bar(&left, &right, width)).style(base), area);
}

/// Lay out a status bar that is exactly `width` columns: keep `right` pinned to
/// the right edge and truncate `left` from its head (keeping the tail, e.g. the
/// file name) when the two would not fit.
fn compose_bar(left: &str, right: &str, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::default();
    }

    let right_w = right.chars().count().min(width);

    let avail = width - right_w;

    let left_chars: Vec<char> = left.chars().collect();

    let left_fitted: String = if left_chars.len() <= avail {
        let pad = avail - left_chars.len();

        format!("{left}{}", " ".repeat(pad))
    } else if avail == 0 {
        String::new()
    } else {
        let tail: String = left_chars[left_chars.len() - (avail - 1)..].iter().collect();

        format!("…{tail}")
    };

    let right_chars: Vec<char> = right.chars().collect();

    let right_fitted: String = right_chars[right_chars.len() - right_w..].iter().collect();

    Line::from(format!("{left_fitted}{right_fitted}"))
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

                let cell_style = if selected { style.bg(SEL_HL) } else { *style };

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
    fn tree_renders_directory() {
        let dir = std::env::temp_dir().join("opencode_tree_test");

        let _ = fs::create_dir_all(&dir);

        fs::write(dir.join("readme.md"), "# hi").unwrap();

        let mut app = App::new(PathBuf::from(&dir), false).unwrap();

        app.picker = None;

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
