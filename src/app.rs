use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{self, Buffer, DiskEvent};
use crate::highlight::SyntaxHighlighter;
use crate::tree::FileTree;

#[derive(PartialEq, Clone, Copy)]
pub enum Focus {
    Tree,

    Editor,
}

pub struct Search {
    pub query: String,
}

pub struct App {
    pub buffer: Option<Buffer>,

    pub tree: FileTree,

    pub highlighter: SyntaxHighlighter,

    pub focus: Focus,

    pub tree_visible: bool,

    pub search: Option<Search>,

    /// When `Some(idx)` the startup style picker is shown with `idx` highlighted;
    /// `None` once a style has been chosen and the editor is active.
    pub picker: Option<usize>,

    pub status: String,

    /// When set, `status` is a success "flash" (green) that auto-clears at this
    /// instant; `None` means a persistent message (or none).
    pub status_until: Option<Instant>,

    pub status_ok: bool,

    pub quit_confirm: bool,

    /// Set after a refused save when the file changed on disk; a second Ctrl+S
    /// then overwrites it.
    pub overwrite_confirm: bool,

    /// A file the user asked to open while the current buffer has unsaved edits;
    /// opening it again confirms discarding those edits.
    pub pending_open: Option<PathBuf>,

    pub should_quit: bool,

    /// Visible editor text rows, updated every render so paging knows the step.
    pub page_rows: usize,

    /// System clipboard handle (`None` if the platform has none); copies also go
    /// to `clip_internal` so paste still works without a system clipboard.
    clipboard: Option<arboard::Clipboard>,

    clip_internal: String,
}

impl App {
    pub fn new(path: PathBuf, force_picker: bool) -> Result<Self> {
        let mut highlighter = SyntaxHighlighter::new();

        let is_dir = path.is_dir();

        let (buffer, tree_root, focus, tree_visible) = if is_dir {
            (None, path.clone(), Focus::Tree, true)
        } else {
            let syntax = highlighter.syntax_name_for_path(&path);

            let buf = Buffer::open(path.clone(), syntax)?;

            let root = buffer::parent_dir(&path);

            (Some(buf), root, Focus::Editor, false)
        };

        // Use the saved style and skip the picker, unless this is the first run
        // or the user explicitly asked to re-pick with `--style`.
        let saved = crate::config::saved_theme().and_then(|name| highlighter.theme_index(&name));

        let picker = match saved {
            Some(idx) => {
                highlighter.set_theme(idx);

                if force_picker { Some(idx) } else { None }
            }

            None => Some(highlighter.current_theme()),
        };

        Ok(Self {
            buffer,
            tree: FileTree::new(tree_root),
            highlighter,
            focus,
            tree_visible,
            search: None,
            picker,
            status: String::new(),
            status_until: None,
            status_ok: false,
            quit_confirm: false,
            overwrite_confirm: false,
            pending_open: None,
            should_quit: false,
            page_rows: 1,
            clipboard: arboard::Clipboard::new().ok(),
            clip_internal: String::new(),
        })
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        if self.picker.is_some() {
            self.on_picker_key(key, ctrl);

            return;
        }

        let is_quit_key = ctrl && key.code == KeyCode::Char('q');

        if !is_quit_key {
            self.quit_confirm = false;
        }

        if !(ctrl && key.code == KeyCode::Char('s')) {
            self.overwrite_confirm = false;
        }

        if self.search.is_some() {
            self.on_search_key(key);

            return;
        }

        if ctrl {
            match key.code {
                KeyCode::Char('q') => return self.request_quit(),

                KeyCode::Char('s') => return self.save(),

                KeyCode::Char('b') => return self.toggle_tree(),

                KeyCode::Char('f') => return self.open_search(),

                KeyCode::Char('c') => return self.copy(),

                KeyCode::Char('x') => return self.cut(),

                KeyCode::Char('v') => return self.paste(),

                KeyCode::Char('a') => return self.select_all(),

                KeyCode::Char('r') => return self.reload(),

                // Other Ctrl combos (Ctrl+arrows, Ctrl+Z/Y, …) fall through to
                // the focused pane.
                _ => {}
            }
        }

        match self.focus {
            Focus::Tree => self.on_tree_key(key),

            Focus::Editor => self.on_editor_key(key),
        }
    }

    fn on_picker_key(&mut self, key: KeyEvent, ctrl: bool) {
        let Some(sel) = self.picker else {
            return;
        };

        if ctrl && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('c')) {
            self.should_quit = true;

            return;
        }

        let count = self.highlighter.theme_count();

        match key.code {
            KeyCode::Up => self.picker = Some(sel.saturating_sub(1)),

            KeyCode::Down => self.picker = Some((sel + 1).min(count - 1)),

            KeyCode::Enter => self.apply_style(sel),

            KeyCode::Esc | KeyCode::Char('q') => self.should_quit = true,

            _ => {}
        }
    }

    fn apply_style(&mut self, idx: usize) {
        self.highlighter.set_theme(idx);

        if let Some(name) = self.highlighter.theme_names().get(idx) {
            let _ = crate::config::save_theme(name);
        }

        if let Some(buf) = self.buffer.as_mut() {
            buf.hl.invalidate(0);
        }

        self.picker = None;
    }

    fn on_tree_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Up => self.tree.move_up(),

            KeyCode::Down => self.tree.move_down(),

            KeyCode::Left => self.tree.collapse(),

            KeyCode::Right => self.tree.expand(),

            KeyCode::Tab => self.focus = Focus::Editor,

            // Enter or Space both open a file / toggle a directory.
            KeyCode::Enter | KeyCode::Char(' ') => self.activate_tree_entry(),

            _ => {}
        }
    }

    fn on_editor_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        let alt = key.modifiers.contains(KeyModifiers::ALT);

        let shift = key.modifiers.contains(KeyModifiers::SHIFT);

        // One navigation modifier per platform — Option on macOS, Ctrl on the
        // rest — so each combo maps to exactly one action (no Ctrl/Alt aliasing).
        let nav = if cfg!(target_os = "macos") { alt } else { ctrl };

        let Some(buf) = self.buffer.as_mut() else {
            if key.code == KeyCode::Tab && self.tree_visible {
                self.focus = Focus::Tree;
            }

            return;
        };

        // `Shift` turns any motion into a selection; without it the motion
        // collapses the selection. `nav` makes the step bigger (word / block).
        match key.code {
            KeyCode::Up => {
                buf.sel(shift);

                if nav { buf.move_para_up() } else { buf.move_up() }
            }

            KeyCode::Down => {
                buf.sel(shift);

                if nav { buf.move_para_down() } else { buf.move_down() }
            }

            KeyCode::Left => {
                buf.sel(shift);

                if nav { buf.move_word_left() } else { buf.move_left() }
            }

            KeyCode::Right => {
                buf.sel(shift);

                if nav { buf.move_word_right() } else { buf.move_right() }
            }

            KeyCode::Home => {
                buf.sel(shift);

                if ctrl { buf.move_doc_start() } else { buf.move_home() }
            }

            KeyCode::End => {
                buf.sel(shift);

                if ctrl { buf.move_doc_end() } else { buf.move_end() }
            }

            KeyCode::PageUp => {
                buf.sel(shift);

                buf.page(-(self.page_rows as isize));
            }

            KeyCode::PageDown => {
                buf.sel(shift);

                buf.page(self.page_rows as isize);
            }

            KeyCode::Esc => buf.clear_selection(),

            KeyCode::Enter => buf.insert_newline(),

            KeyCode::Backspace if nav => buf.delete_word_left(),

            KeyCode::Backspace => buf.backspace(),

            KeyCode::Delete => buf.delete(),

            // Undo / redo (Ctrl+Z, Ctrl+Y, Ctrl+Shift+Z). macOS terminals do not
            // forward Cmd, so Ctrl is used on both platforms.
            KeyCode::Char('z') | KeyCode::Char('Z') if ctrl && shift => buf.redo(),

            KeyCode::Char('z') | KeyCode::Char('Z') if ctrl => buf.undo(),

            KeyCode::Char('y') if ctrl => buf.redo(),

            // macOS terminals with Option-as-Meta / "Natural Text Editing" send
            // ESC-b / ESC-f for Option+←/→ (and the upper-case form when Shift is
            // held). Map them to word motion so Option+arrow works there too.
            KeyCode::Char('b') if alt => {
                buf.sel(false);

                buf.move_word_left();
            }

            KeyCode::Char('B') if alt => {
                buf.sel(true);

                buf.move_word_left();
            }

            KeyCode::Char('f') if alt => {
                buf.sel(false);

                buf.move_word_right();
            }

            KeyCode::Char('F') if alt => {
                buf.sel(true);

                buf.move_word_right();
            }

            // Shift+Tab outdents the current line (Tab below indents).
            KeyCode::BackTab => buf.outdent_line(),

            KeyCode::Tab => {
                if self.tree_visible {
                    self.focus = Focus::Tree;
                } else {
                    buf.indent_line();
                }
            }

            KeyCode::Char(c) if !ctrl && !alt => buf.insert_char(c),

            _ => {}
        }
    }

    fn on_search_key(&mut self, key: KeyEvent) {
        let Some(search) = self.search.as_mut() else {
            return;
        };

        match key.code {
            KeyCode::Esc => {
                self.search = None;

                self.clear_flash();
            }

            KeyCode::Backspace => {
                search.query.pop();
            }

            KeyCode::Enter => self.find_next(),

            KeyCode::Char(c) => search.query.push(c),

            _ => {}
        }
    }

    fn activate_tree_entry(&mut self) {
        let Some(node) = self.tree.selected_node() else {
            return;
        };

        if node.is_dir {
            if node.expanded {
                self.tree.collapse();
            } else {
                self.tree.expand();
            }

            return;
        }

        let path = node.path.clone();

        let dirty = self.buffer.as_ref().map(|b| b.modified).unwrap_or(false);

        // Switching files must never silently lose edits: warn once, then let a
        // second Enter on the same file discard them (or Ctrl+S to keep them).
        if dirty && self.pending_open.as_deref() != Some(path.as_path()) {
            self.pending_open = Some(path);

            self.set_error("Unsaved changes — Ctrl+S to save, or Enter again to discard & open".to_string());

            return;
        }

        self.open_file(path);
    }

    fn open_file(&mut self, path: PathBuf) {
        let syntax = self.highlighter.syntax_name_for_path(&path);

        match Buffer::open(path, syntax) {
            Ok(buf) => {
                self.buffer = Some(buf);

                self.focus = Focus::Editor;

                // Close the file list so the editor goes full-screen; Ctrl+B
                // brings it back to pick another file.
                self.tree_visible = false;

                self.pending_open = None;

                self.clear_flash();
            }

            Err(e) => self.set_error(format!("Error: {e}")),
        }
    }

    fn save(&mut self) {
        let Some(buf) = self.buffer.as_mut() else {
            return;
        };

        // Guard against clobbering an external change with a single keystroke.
        if buf.disk_changed && !self.overwrite_confirm {
            self.overwrite_confirm = true;

            self.set_error(
                "⚠ Changed on disk — Ctrl+S again to overwrite, or Ctrl+R to reload".to_string(),
            );

            return;
        }

        match buf.save() {
            Ok(()) => {
                let name = buf.file_name();

                self.overwrite_confirm = false;

                self.pending_open = None;

                self.flash(format!("✓ Saved {name}"));
            }

            Err(e) => self.set_error(format!("Error: {e}")),
        }
    }

    fn reload(&mut self) {
        let Some(buf) = self.buffer.as_mut() else {
            return;
        };

        match buf.reload_from_disk() {
            Ok(()) => self.flash("↻ Reloaded from disk".to_string()),

            Err(e) => self.set_error(format!("Error: {e}")),
        }
    }

    /// Check the open file against disk (called on idle ticks). Clean buffers
    /// auto-reload; the conflict warning is derived from `buf.disk_changed`.
    fn poll_disk(&mut self) {
        let Some(buf) = self.buffer.as_mut() else {
            return;
        };

        if buf.poll_disk() == DiskEvent::Reloaded {
            self.flash("↻ Reloaded from disk".to_string());
        }
    }

    /// How long the run loop should wait before waking itself: the sooner of a
    /// fading flash and the disk-poll interval (only while a file is open).
    pub fn wake_after(&self) -> Option<Duration> {
        let mut wake: Option<Duration> = None;

        if let Some(until) = self.status_until {
            wake = Some(until.saturating_duration_since(Instant::now()));
        }

        if self.buffer.is_some() {
            let poll = Duration::from_millis(1000);

            wake = Some(wake.map_or(poll, |w| w.min(poll)));
        }

        wake
    }

    /// Idle housekeeping done when the loop wakes without a keypress.
    pub fn tick(&mut self) {
        if let Some(until) = self.status_until {
            if Instant::now() >= until {
                self.clear_flash();
            }
        }

        self.poll_disk();
    }

    fn copy(&mut self) {
        let Some(text) = self.buffer.as_ref().and_then(|b| b.selected_text()) else {
            self.set_error("Nothing selected".to_string());

            return;
        };

        self.set_clipboard(text);

        self.flash("Copied".to_string());
    }

    fn cut(&mut self) {
        let Some(buf) = self.buffer.as_mut() else {
            return;
        };

        let Some(text) = buf.selected_text() else {
            self.set_error("Nothing selected".to_string());

            return;
        };

        buf.delete_selection();

        self.set_clipboard(text);

        self.flash("Cut".to_string());
    }

    fn paste(&mut self) {
        let text = self.read_clipboard();

        if let Some(buf) = self.buffer.as_mut() {
            buf.insert_str(&text);
        }
    }

    fn select_all(&mut self) {
        if let Some(buf) = self.buffer.as_mut() {
            buf.select_all();
        }
    }

    fn set_clipboard(&mut self, text: String) {
        if let Some(cb) = self.clipboard.as_mut() {
            let _ = cb.set_text(text.clone());
        }

        self.clip_internal = text;
    }

    fn read_clipboard(&mut self) -> String {
        if let Some(cb) = self.clipboard.as_mut() {
            if let Ok(text) = cb.get_text() {
                return text;
            }
        }

        self.clip_internal.clone()
    }

    /// A green success message that disappears on its own after a moment.
    fn flash(&mut self, msg: String) {
        self.status = msg;

        self.status_ok = true;

        self.status_until = Some(Instant::now() + Duration::from_millis(1500));
    }

    /// A message that stays until the next action (e.g. an error).
    fn set_error(&mut self, msg: String) {
        self.status = msg;

        self.status_ok = false;

        self.status_until = None;
    }

    pub fn clear_flash(&mut self) {
        self.status.clear();

        self.status_ok = false;

        self.status_until = None;
    }

    /// Ctrl+B as a three-step cycle so one key both opens the file list and
    /// returns to it: hidden → show & focus → (from editor) focus it → hide.
    fn toggle_tree(&mut self) {
        if !self.tree_visible {
            self.tree_visible = true;

            self.focus = Focus::Tree;
        } else if self.focus != Focus::Tree {
            self.focus = Focus::Tree;
        } else {
            self.tree_visible = false;

            self.focus = Focus::Editor;
        }
    }

    fn open_search(&mut self) {
        if self.buffer.is_some() {
            self.search = Some(Search {
                query: String::new(),
            });
        }
    }

    fn find_next(&mut self) {
        let Some(query) = self.search.as_ref().map(|s| s.query.clone()) else {
            return;
        };

        let Some(buf) = self.buffer.as_mut() else {
            return;
        };

        let from = buf.char_idx() + 1;

        let found = buffer::find_next(&buf.rope, &query, from);

        if let Some(idx) = found {
            buf.move_to_char(idx);
        }

        match found {
            Some(_) => self.flash(format!("Found '{query}'")),

            None => self.set_error(format!("Not found: '{query}'")),
        }
    }

    fn request_quit(&mut self) {
        let dirty = self.buffer.as_ref().map(|b| b.modified).unwrap_or(false);

        if dirty && !self.quit_confirm {
            self.quit_confirm = true;

            self.set_error("Unsaved changes — Ctrl+Q again to quit, or Ctrl+S to save".to_string());
        } else {
            self.should_quit = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn app_with(name: &str, text: &str) -> App {
        let path = std::env::temp_dir().join(format!("opencode_app_{name}.txt"));

        fs::write(&path, text).unwrap();

        let mut app = App::new(path, false).unwrap();

        app.picker = None;

        app
    }

    fn plain(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    /// The platform navigation modifier the editor actually listens to.
    fn nav(code: KeyCode) -> KeyEvent {
        let m = if cfg!(target_os = "macos") {
            KeyModifiers::ALT
        } else {
            KeyModifiers::CONTROL
        };

        KeyEvent::new(code, m)
    }

    fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    fn alt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    #[test]
    fn meta_b_f_jump_by_word() {
        // What macOS terminals send for Option+←/→ (Option-as-Meta).
        let mut app = app_with("metaword", "foo bar");

        app.on_key(alt(KeyCode::Char('f')));

        assert_eq!(app.buffer.as_ref().unwrap().cursor_col, 3);

        app.on_key(alt(KeyCode::Char('f')));

        assert_eq!(app.buffer.as_ref().unwrap().cursor_col, 7);

        app.on_key(alt(KeyCode::Char('b')));

        assert_eq!(app.buffer.as_ref().unwrap().cursor_col, 4);

        assert!(!app.buffer.as_ref().unwrap().modified, "meta motion must not type letters");
    }

    #[test]
    fn typing_inserts_and_marks_modified() {
        let mut app = app_with("type", "ab cd");

        app.on_key(plain(KeyCode::Char('X')));

        let buf = app.buffer.as_ref().unwrap();

        assert!(buf.modified);

        assert_eq!(buf.rope.to_string(), "Xab cd");
    }

    #[test]
    fn ctrl_letter_command_does_not_type_text() {
        let mut app = app_with("ctrlletter", "ab");

        // An unbound Ctrl+letter must never insert that letter.
        app.on_key(ctrl(KeyCode::Char('g')));

        let buf = app.buffer.as_ref().unwrap();

        assert!(!buf.modified);

        assert_eq!(buf.rope.to_string(), "ab");
    }

    #[test]
    fn home_end_are_line_motions() {
        let mut app = app_with("linemo", "hello world");

        app.on_key(plain(KeyCode::End));

        assert_eq!(app.buffer.as_ref().unwrap().cursor_col, 11);

        app.on_key(plain(KeyCode::Home));

        assert_eq!(app.buffer.as_ref().unwrap().cursor_col, 0);
    }

    #[test]
    fn ctrl_a_selects_all() {
        let mut app = app_with("selectall", "line one\nline two\n");

        app.on_key(ctrl(KeyCode::Char('a')));

        assert_eq!(
            app.buffer.as_ref().unwrap().selected_text().as_deref(),
            Some("line one\nline two\n")
        );
    }

    #[test]
    fn nav_up_reaches_doc_top_without_fn() {
        let mut app = app_with("doctop", "one\ntwo\nthree");

        app.buffer.as_mut().unwrap().cursor_line = 2;

        app.on_key(nav(KeyCode::Up)); // block jump with no blank line lands at top

        assert_eq!(app.buffer.as_ref().unwrap().cursor_line, 0);
    }

    #[test]
    fn nav_right_jumps_by_word() {
        let mut app = app_with("navword", "foo bar");

        app.on_key(nav(KeyCode::Right));

        assert_eq!(app.buffer.as_ref().unwrap().cursor_col, 3);
    }

    #[test]
    fn nav_up_down_jumps_blocks() {
        let mut app = app_with("blocks", "a\nb\n\nc\n");

        app.on_key(nav(KeyCode::Down));

        assert_eq!(app.buffer.as_ref().unwrap().cursor_line, 2);
    }

    #[test]
    fn ctrl_q_guards_quit_when_modified() {
        let mut app = app_with("guard", "data");

        app.on_key(plain(KeyCode::Char('!')));

        app.on_key(ctrl(KeyCode::Char('q')));

        assert!(!app.should_quit);

        assert!(app.quit_confirm);

        app.on_key(ctrl(KeyCode::Char('q')));

        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_q_quits_immediately_when_clean() {
        let mut app = app_with("clean", "data");

        app.on_key(ctrl(KeyCode::Char('q')));

        assert!(app.should_quit);
    }

    #[test]
    fn shift_arrow_selects_and_ctrl_c_copies() {
        let mut app = app_with("selcopy", "hello world");

        app.clipboard = None; // use the internal clipboard; don't touch the OS one

        app.on_key(shift(KeyCode::Right));

        app.on_key(shift(KeyCode::Right));

        let buf = app.buffer.as_ref().unwrap();

        assert_eq!(buf.selected_text().as_deref(), Some("he"));

        app.on_key(ctrl(KeyCode::Char('c'))); // copy (no longer quit)

        assert!(!app.should_quit, "Ctrl+C must copy, not quit");

        assert_eq!(app.clip_internal, "he");
    }

    #[test]
    fn typing_over_selection_replaces_it() {
        let mut app = app_with("replace", "abcde");

        app.on_key(shift(KeyCode::Right));

        app.on_key(shift(KeyCode::Right));

        app.on_key(plain(KeyCode::Char('X')));

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), "Xcde");
    }

    #[test]
    fn cut_then_paste_round_trips() {
        let mut app = app_with("cutpaste", "abcde");

        app.clipboard = None; // deterministic: round-trip through the internal clipboard

        app.on_key(shift(KeyCode::Right));

        app.on_key(shift(KeyCode::Right)); // select "ab"

        app.on_key(ctrl(KeyCode::Char('x'))); // cut -> "cde"

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), "cde");

        app.on_key(plain(KeyCode::End)); // move to end of line

        app.on_key(ctrl(KeyCode::Char('v'))); // paste "ab"

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), "cdeab");
    }

    #[test]
    fn ctrl_b_cycles_tree_open_focus_hide() {
        let mut app = app_with("tree", "x");

        assert!(!app.tree_visible);

        app.on_key(ctrl(KeyCode::Char('b')));

        assert!(app.tree_visible && app.focus == Focus::Tree);

        app.focus = Focus::Editor;

        app.on_key(ctrl(KeyCode::Char('b')));

        assert!(app.tree_visible && app.focus == Focus::Tree);

        app.on_key(ctrl(KeyCode::Char('b')));

        assert!(!app.tree_visible && app.focus == Focus::Editor);
    }

    #[test]
    fn tab_indents_backtab_outdents() {
        let mut app = app_with("indentkey", "foo");

        app.on_key(plain(KeyCode::Tab));

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), "    foo");

        app.on_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE));

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), "foo");
    }

    #[test]
    fn shift_arrow_grows_selection_plain_arrow_clears_it() {
        let mut app = app_with("selclear", "hello");

        app.on_key(shift(KeyCode::Right));

        app.on_key(shift(KeyCode::Right));

        assert_eq!(app.buffer.as_ref().unwrap().selected_text().as_deref(), Some("he"));

        app.on_key(plain(KeyCode::Right)); // a plain move collapses the selection

        assert!(app.buffer.as_ref().unwrap().selection().is_none());
    }

    #[test]
    fn plain_angle_bracket_is_inserted() {
        let mut app = app_with("angle", "");

        app.on_key(plain(KeyCode::Char('>')));

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), ">");
    }

    #[test]
    fn switching_files_warns_then_discards() {
        let dir = std::env::temp_dir().join("opencode_switch_test");

        let _ = fs::remove_dir_all(&dir);

        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join("a.txt"), "aaa").unwrap();

        fs::write(dir.join("b.txt"), "bbb").unwrap();

        let mut app = App::new(dir.clone(), false).unwrap();

        app.picker = None;

        app.on_key(plain(KeyCode::Enter)); // open a.txt (selected first)

        app.on_key(plain(KeyCode::Char('X'))); // make it dirty

        assert!(app.buffer.as_ref().unwrap().modified);

        app.on_key(ctrl(KeyCode::Char('b'))); // focus the tree

        app.on_key(plain(KeyCode::Down)); // select b.txt

        app.on_key(plain(KeyCode::Enter)); // first Enter: warn, do not switch

        assert!(app.buffer.as_ref().unwrap().rope.to_string().contains("aaa"));

        assert!(app.status.contains("Unsaved"));

        app.on_key(plain(KeyCode::Enter)); // second Enter: discard & open b

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), "bbb");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn space_opens_file_in_tree() {
        let dir = std::env::temp_dir().join("opencode_space_test");

        let _ = fs::remove_dir_all(&dir);

        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join("a.txt"), "hello").unwrap();

        let mut app = App::new(dir.clone(), false).unwrap();

        app.picker = None;

        assert!(app.buffer.is_none());

        app.on_key(plain(KeyCode::Char(' '))); // Space opens the selected file

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), "hello");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn opening_a_file_from_tree_hides_it() {
        let dir = std::env::temp_dir().join("opencode_hidetree_test");

        let _ = fs::remove_dir_all(&dir);

        fs::create_dir_all(&dir).unwrap();

        fs::write(dir.join("a.txt"), "hi").unwrap();

        let mut app = App::new(dir.clone(), false).unwrap();

        app.picker = None;

        assert!(app.tree_visible, "a directory launch shows the tree");

        app.on_key(plain(KeyCode::Enter)); // open a.txt

        assert!(!app.tree_visible, "tree closes after opening a file");

        assert!(app.buffer.is_some());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_conflict_needs_a_second_ctrl_s() {
        let path = std::env::temp_dir().join("opencode_app_conflict.txt");

        fs::write(&path, "v1\n").unwrap();

        let mut app = App::new(path.clone(), false).unwrap();

        app.picker = None;

        app.on_key(plain(KeyCode::Char('X'))); // dirty: "Xv1\n"

        fs::write(&path, "v2\n").unwrap(); // external change

        app.buffer.as_mut().unwrap().disk_changed = true; // (detection covered by buffer tests)

        app.on_key(ctrl(KeyCode::Char('s'))); // refused, arms overwrite

        assert!(app.overwrite_confirm);

        assert_eq!(fs::read_to_string(&path).unwrap(), "v2\n"); // not written yet

        app.on_key(ctrl(KeyCode::Char('s'))); // overwrite

        assert!(!app.buffer.as_ref().unwrap().disk_changed);

        assert_eq!(fs::read_to_string(&path).unwrap(), "Xv1\n");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn ctrl_r_reloads_from_disk() {
        let path = std::env::temp_dir().join("opencode_app_reload.txt");

        fs::write(&path, "v1\n").unwrap();

        let mut app = App::new(path.clone(), false).unwrap();

        app.picker = None;

        app.on_key(plain(KeyCode::Char('X')));

        fs::write(&path, "v2\n").unwrap();

        app.buffer.as_mut().unwrap().disk_changed = true;

        app.on_key(ctrl(KeyCode::Char('r'))); // reload from disk

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), "v2\n");

        assert!(!app.buffer.as_ref().unwrap().disk_changed);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn ctrl_z_undoes_and_ctrl_y_redoes() {
        let mut app = app_with("undokey", "");

        app.on_key(plain(KeyCode::Char('h')));

        app.on_key(plain(KeyCode::Char('i')));

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), "hi");

        app.on_key(ctrl(KeyCode::Char('z')));

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), "");

        app.on_key(ctrl(KeyCode::Char('y')));

        assert_eq!(app.buffer.as_ref().unwrap().rope.to_string(), "hi");
    }
}
