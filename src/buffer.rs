use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ropey::Rope;

use crate::highlight::HlCache;

const INDENT: &str = "    ";
const UNDO_LIMIT: usize = 600;

/// Kind of the last edit, used to coalesce undo steps: a run of plain typing or
/// a run of deletes folds into one undoable group; everything else stands alone.
#[derive(PartialEq, Clone, Copy)]
enum EditKind {
    Insert,

    Delete,

    Other,
}

#[derive(Clone)]
struct Snapshot {
    rope: Rope,

    cursor_line: usize,

    cursor_col: usize,
}

/// A single open text file: the rope, the cursor, viewport scroll offsets, its
/// highlight cache and undo history. All editing goes through this type so the
/// highlight cache and undo stack stay in sync with the text.
pub struct Buffer {
    pub rope: Rope,

    pub path: PathBuf,

    pub cursor_line: usize,

    pub cursor_col: usize,

    pub desired_col: usize,

    pub scroll_row: usize,

    pub scroll_col: usize,

    /// `true` only when the current text differs from `saved` — editing then
    /// undoing back to the saved state clears it.
    pub modified: bool,

    pub hl: HlCache,

    /// Snapshot of the text as last written to disk (or as opened). Compared
    /// against `rope` to decide `modified`. Clone is O(1) in ropey.
    saved: Rope,

    undo_stack: Vec<Snapshot>,

    redo_stack: Vec<Snapshot>,

    last_edit: Option<EditKind>,

    last_edit_pos: Option<usize>,

    last_insert_char: Option<char>,

    /// Fixed end of the selection (char index); the cursor is the moving end.
    /// `None` means no active selection.
    anchor: Option<usize>,
}

impl Buffer {
    pub fn open(path: PathBuf, syntax_name: String) -> Result<Self> {
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,

            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),

            Err(e) => {
                return Err(e).with_context(|| format!("reading {}", path.display()));
            }
        };

        let rope = Rope::from_str(&text);

        Ok(Self {
            saved: rope.clone(),
            rope,
            path,
            cursor_line: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            scroll_col: 0,
            modified: false,
            hl: HlCache::new(syntax_name),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit: None,
            last_edit_pos: None,
            last_insert_char: None,
            anchor: None,
        })
    }

    /// Begin or extend a selection (call before a cursor move): anchors at the
    /// current cursor if there isn't a selection yet, so the moving cursor
    /// sweeps a range. With `selecting == false` the selection is collapsed.
    pub fn sel(&mut self, selecting: bool) {
        if selecting {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor_idx());
            }
        } else {
            self.anchor = None;
        }
    }

    pub fn clear_selection(&mut self) {
        self.anchor = None;
    }

    /// The selected char range `(start, end)` with `start <= end`, or `None`
    /// when there is no selection (no anchor, or anchor == cursor).
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;

        let cursor = self.cursor_idx();

        if anchor == cursor {
            None
        } else {
            Some((anchor.min(cursor), anchor.max(cursor)))
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection()?;

        Some(self.rope.slice(start..end).to_string())
    }

    /// Delete the active selection (if any). Returns `true` when something was
    /// removed, leaving the cursor at the start of the former selection.
    pub fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };

        let line = self.rope.char_to_line(start);

        self.record(EditKind::Other, start, true);

        self.rope.remove(start..end);

        self.anchor = None;

        self.move_to_char(start);

        self.hl.invalidate(line);

        self.refresh_modified();

        self.mark_edit(None);

        true
    }

    /// The selected column range within `line` (char columns), for rendering;
    /// `None` if the line has no selected cells.
    pub fn selection_for_line(&self, line: usize) -> Option<(usize, usize)> {
        let (start, end) = self.selection()?;

        let line_start = self.rope.line_to_char(line);

        let line_end = line_start + self.rope.line(line).len_chars();

        let from = start.max(line_start);

        let to = end.min(line_end);

        if from >= to {
            None
        } else {
            Some((from - line_start, to - line_start))
        }
    }

    /// Recompute `modified` by comparing against the last-saved text. ropey's
    /// equality short-circuits on length, so this is O(1) unless the lengths
    /// match (e.g. after undoing back to the saved content).
    fn refresh_modified(&mut self) {
        self.modified = self.rope != self.saved;
    }

    /// Record the pre-edit state for undo. An edit coalesces with the previous
    /// one only when it is the same kind, contiguous (the cursor is exactly
    /// where the last edit left it) and not forced to break — so a new word, a
    /// cursor jump or a structural edit each start a fresh undo step.
    fn record(&mut self, kind: EditKind, pos: usize, force_break: bool) {
        let coalesce = !force_break
            && kind != EditKind::Other
            && self.last_edit == Some(kind)
            && self.last_edit_pos == Some(pos);

        if !coalesce {
            self.undo_stack.push(self.snapshot());

            if self.undo_stack.len() > UNDO_LIMIT {
                self.undo_stack.remove(0);
            }

            self.redo_stack.clear();
        }

        self.last_edit = Some(kind);
    }

    fn mark_edit(&mut self, last_char: Option<char>) {
        self.last_edit_pos = Some(self.cursor_idx());

        self.last_insert_char = last_char;
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            rope: self.rope.clone(),
            cursor_line: self.cursor_line,
            cursor_col: self.cursor_col,
        }
    }

    fn restore(&mut self, snap: Snapshot) {
        self.rope = snap.rope;

        self.cursor_line = snap.cursor_line.min(self.last_line());

        self.cursor_col = snap.cursor_col.min(self.line_len(self.cursor_line));

        self.desired_col = self.cursor_col;

        self.hl.invalidate(0);

        self.refresh_modified();

        self.last_edit = None;

        self.last_edit_pos = None;

        self.last_insert_char = None;
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo_stack.pop() {
            self.redo_stack.push(self.snapshot());

            self.restore(prev);
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo_stack.pop() {
            self.undo_stack.push(self.snapshot());

            self.restore(next);
        }
    }

    pub fn save(&mut self) -> Result<()> {
        let text = self.rope.to_string();

        fs::write(&self.path, text).with_context(|| format!("writing {}", self.path.display()))?;

        self.saved = self.rope.clone();

        self.modified = false;

        Ok(())
    }

    pub fn last_line(&self) -> usize {
        self.rope.len_lines().saturating_sub(1)
    }

    /// Length of a line in chars, excluding the trailing line break.
    pub fn line_len(&self, line: usize) -> usize {
        let slice = self.rope.line(line);

        let mut len = slice.len_chars();

        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;

            if len > 0 && slice.char(len - 1) == '\r' {
                len -= 1;
            }
        }

        len
    }

    fn cursor_idx(&self) -> usize {
        self.rope.line_to_char(self.cursor_line) + self.cursor_col
    }

    fn current_indent(&self) -> String {
        self.rope
            .line(self.cursor_line)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect()
    }

    pub fn insert_char(&mut self, ch: char) {
        // Typing over a selection replaces it.
        self.delete_selection();

        let idx = self.cursor_idx();

        // Start a new undo step at the beginning of each word so a long burst of
        // typing undoes word by word, not all at once.
        let word_start = is_word_char(ch) && self.last_insert_char.is_none_or(|p| !is_word_char(p));

        self.record(EditKind::Insert, idx, word_start);

        self.rope.insert_char(idx, ch);

        self.cursor_col += 1;

        self.desired_col = self.cursor_col;

        self.hl.invalidate(self.cursor_line);

        self.refresh_modified();

        self.mark_edit(Some(ch));
    }

    /// Insert a block of text at the cursor (used by paste), replacing the
    /// selection first if there is one. The cursor lands after the text.
    pub fn insert_str(&mut self, text: &str) {
        self.delete_selection();

        if text.is_empty() {
            return;
        }

        let idx = self.cursor_idx();

        let line = self.rope.char_to_line(idx);

        self.record(EditKind::Other, idx, true);

        self.rope.insert(idx, text);

        self.move_to_char(idx + text.chars().count());

        self.hl.invalidate(line);

        self.refresh_modified();

        self.mark_edit(None);
    }

    /// Indent the whole current line by one level, regardless of cursor column.
    pub fn indent_line(&mut self) {
        let start = self.rope.line_to_char(self.cursor_line);

        self.record(EditKind::Other, start, true);

        self.rope.insert(start, INDENT);

        self.cursor_col += INDENT.chars().count();

        self.desired_col = self.cursor_col;

        self.hl.invalidate(self.cursor_line);

        self.refresh_modified();

        self.mark_edit(None);
    }

    /// Remove one indent level (a leading tab, or up to four spaces) from the
    /// current line.
    pub fn outdent_line(&mut self) {
        let start = self.rope.line_to_char(self.cursor_line);

        let len = self.line_len(self.cursor_line);

        if len == 0 {
            return;
        }

        let remove = if self.rope.char(start) == '\t' {
            1
        } else {
            let mut n = 0;

            while n < INDENT.len() && n < len && self.rope.char(start + n) == ' ' {
                n += 1;
            }

            n
        };

        if remove > 0 {
            self.record(EditKind::Other, start, true);

            self.rope.remove(start..start + remove);

            self.cursor_col = self.cursor_col.saturating_sub(remove);

            self.desired_col = self.cursor_col;

            self.hl.invalidate(self.cursor_line);

            self.refresh_modified();

            self.mark_edit(None);
        }
    }

    pub fn insert_newline(&mut self) {
        self.delete_selection();

        let idx = self.cursor_idx();

        self.record(EditKind::Other, idx, true);

        let indent = self.current_indent();

        self.rope.insert_char(idx, '\n');

        if !indent.is_empty() {
            self.rope.insert(idx + 1, &indent);
        }

        self.hl.invalidate(self.cursor_line);

        self.cursor_line += 1;

        self.cursor_col = indent.chars().count();

        self.desired_col = self.cursor_col;

        self.refresh_modified();

        self.mark_edit(None);
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            return;
        }

        let idx = self.cursor_idx();

        if idx == 0 {
            return;
        }

        self.record(EditKind::Delete, idx, false);

        if self.cursor_col > 0 {
            self.rope.remove(idx - 1..idx);

            self.cursor_col -= 1;

            self.hl.invalidate(self.cursor_line);
        } else {
            let prev = self.cursor_line - 1;

            let prev_len = self.line_len(prev);

            self.rope.remove(idx - 1..idx);

            self.cursor_line = prev;

            self.cursor_col = prev_len;

            self.hl.invalidate(prev);
        }

        self.desired_col = self.cursor_col;

        self.refresh_modified();

        self.mark_edit(None);
    }

    pub fn delete(&mut self) {
        if self.delete_selection() {
            return;
        }

        let idx = self.cursor_idx();

        if idx < self.rope.len_chars() {
            self.record(EditKind::Delete, idx, false);

            self.rope.remove(idx..idx + 1);

            self.hl.invalidate(self.cursor_line);

            self.refresh_modified();

            self.mark_edit(None);
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            self.cursor_line -= 1;

            self.cursor_col = self.line_len(self.cursor_line);
        }

        self.desired_col = self.cursor_col;
    }

    pub fn move_right(&mut self) {
        let len = self.line_len(self.cursor_line);

        if self.cursor_col < len {
            self.cursor_col += 1;
        } else if self.cursor_line < self.last_line() {
            self.cursor_line += 1;

            self.cursor_col = 0;
        }

        self.desired_col = self.cursor_col;
    }

    pub fn move_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;

            self.cursor_col = self.desired_col.min(self.line_len(self.cursor_line));
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor_line < self.last_line() {
            self.cursor_line += 1;

            self.cursor_col = self.desired_col.min(self.line_len(self.cursor_line));
        }
    }

    pub fn move_home(&mut self) {
        self.cursor_col = 0;

        self.desired_col = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor_col = self.line_len(self.cursor_line);

        self.desired_col = self.cursor_col;
    }

    pub fn page(&mut self, delta: isize) {
        let last = self.last_line() as isize;

        let target = (self.cursor_line as isize + delta).clamp(0, last);

        self.cursor_line = target as usize;

        self.cursor_col = self.desired_col.min(self.line_len(self.cursor_line));
    }

    /// Move the cursor to an absolute char index (used by search).
    pub fn move_to_char(&mut self, idx: usize) {
        let idx = idx.min(self.rope.len_chars());

        let line = self.rope.char_to_line(idx);

        self.cursor_line = line;

        self.cursor_col = idx - self.rope.line_to_char(line);

        self.desired_col = self.cursor_col;
    }

    pub fn move_doc_start(&mut self) {
        self.move_to_char(0);
    }

    pub fn move_doc_end(&mut self) {
        self.move_to_char(self.rope.len_chars());
    }

    pub fn move_word_left(&mut self) {
        let target = self.word_left_from(self.cursor_idx());

        self.move_to_char(target);
    }

    pub fn move_word_right(&mut self) {
        let target = self.word_right_from(self.cursor_idx());

        self.move_to_char(target);
    }

    pub fn delete_word_left(&mut self) {
        let end = self.cursor_idx();

        let start = self.word_left_from(end);

        if start < end {
            let line = self.rope.char_to_line(start);

            self.record(EditKind::Other, end, true);

            self.rope.remove(start..end);

            self.move_to_char(start);

            self.hl.invalidate(line);

            self.refresh_modified();

            self.mark_edit(None);
        }
    }

    /// Jump to the previous blank line (paragraph / block boundary).
    pub fn move_para_up(&mut self) {
        let mut l = self.cursor_line.saturating_sub(1);

        while l > 0 && self.line_is_blank(l) {
            l -= 1;
        }

        while l > 0 && !self.line_is_blank(l) {
            l -= 1;
        }

        self.set_line(l);
    }

    /// Jump to the next blank line (paragraph / block boundary).
    pub fn move_para_down(&mut self) {
        let last = self.last_line();

        let mut l = (self.cursor_line + 1).min(last);

        while l < last && self.line_is_blank(l) {
            l += 1;
        }

        while l < last && !self.line_is_blank(l) {
            l += 1;
        }

        self.set_line(l);
    }

    fn set_line(&mut self, line: usize) {
        self.cursor_line = line.min(self.last_line());

        self.cursor_col = 0;

        self.desired_col = 0;
    }

    fn line_is_blank(&self, line: usize) -> bool {
        self.rope.line(line).chars().all(|c| c.is_whitespace())
    }

    /// First char index to the left of `from` that begins the current word,
    /// skipping any leading whitespace. Words are runs of one character class.
    fn word_left_from(&self, from: usize) -> usize {
        let mut i = from;

        while i > 0 && self.rope.char(i - 1).is_whitespace() {
            i -= 1;
        }

        if i > 0 {
            let class = char_class(self.rope.char(i - 1));

            while i > 0 {
                let c = self.rope.char(i - 1);

                if c.is_whitespace() || char_class(c) != class {
                    break;
                }

                i -= 1;
            }
        }

        i
    }

    /// First char index to the right of `from` past the current word.
    fn word_right_from(&self, from: usize) -> usize {
        let n = self.rope.len_chars();

        let mut i = from;

        while i < n && self.rope.char(i).is_whitespace() {
            i += 1;
        }

        if i < n {
            let class = char_class(self.rope.char(i));

            while i < n {
                let c = self.rope.char(i);

                if c.is_whitespace() || char_class(c) != class {
                    break;
                }

                i += 1;
            }
        }

        i
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("untitled")
            .to_string()
    }

    pub fn char_idx(&self) -> usize {
        self.cursor_idx()
    }
}

/// Find the next occurrence of `needle` at or after `from` (char index),
/// wrapping around to the start. Returns the char index of the match.
pub fn find_next(rope: &Rope, needle: &str, from: usize) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }

    let text = rope.to_string();

    let from_byte = char_to_byte(&text, from.min(text.chars().count()));

    if let Some(rel) = text[from_byte..].find(needle) {
        return Some(byte_to_char(&text, from_byte + rel));
    }

    text[..from_byte]
        .find(needle)
        .map(|b| byte_to_char(&text, b))
}

/// `0` is unused (whitespace is handled separately); word characters and other
/// punctuation form two distinct classes so word motion stops at boundaries.
fn char_class(c: char) -> u8 {
    if is_word_char(c) {
        1
    } else {
        2
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

fn byte_to_char(s: &str, byte_idx: usize) -> usize {
    s[..byte_idx].chars().count()
}

#[allow(dead_code)]
pub fn parent_dir(path: &Path) -> PathBuf {
    path.parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::highlight::HlCache;

    fn buf(text: &str) -> Buffer {
        let rope = Rope::from_str(text);

        Buffer {
            saved: rope.clone(),
            rope,
            path: PathBuf::from("test.rs"),
            cursor_line: 0,
            cursor_col: 0,
            desired_col: 0,
            scroll_row: 0,
            scroll_col: 0,
            modified: false,
            hl: HlCache::new("Plain Text".to_string()),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit: None,
            last_edit_pos: None,
            last_insert_char: None,
            anchor: None,
        }
    }

    #[test]
    fn line_metrics() {
        let b = buf("abc\nde\n");

        assert_eq!(b.last_line(), 2);

        assert_eq!(b.line_len(0), 3);

        assert_eq!(b.line_len(1), 2);

        assert_eq!(b.line_len(2), 0);
    }

    #[test]
    fn insert_and_newline_autoindent() {
        let mut b = buf("    foo");

        b.cursor_col = 7;

        b.insert_newline();

        assert_eq!(b.cursor_line, 1);

        assert_eq!(b.cursor_col, 4);

        assert_eq!(b.rope.to_string(), "    foo\n    ");
    }

    #[test]
    fn backspace_merges_lines() {
        let mut b = buf("ab\ncd");

        b.cursor_line = 1;

        b.cursor_col = 0;

        b.backspace();

        assert_eq!(b.cursor_line, 0);

        assert_eq!(b.cursor_col, 2);

        assert_eq!(b.rope.to_string(), "abcd");
    }

    #[test]
    fn backspace_within_line() {
        let mut b = buf("abc");

        b.cursor_col = 2;

        b.backspace();

        assert_eq!(b.cursor_col, 1);

        assert_eq!(b.rope.to_string(), "ac");
    }

    #[test]
    fn delete_forward_merges() {
        let mut b = buf("ab\ncd");

        b.cursor_col = 2;

        b.delete();

        assert_eq!(b.rope.to_string(), "abcd");

        assert_eq!(b.cursor_line, 0);
    }

    #[test]
    fn vertical_move_keeps_desired_col() {
        let mut b = buf("longline\nx\nanother");

        b.cursor_col = 6;

        b.desired_col = 6;

        b.move_down();

        assert_eq!(b.cursor_col, 1);

        b.move_down();

        assert_eq!(b.cursor_col, 6);
    }

    #[test]
    fn word_motion() {
        let mut b = buf("foo bar  baz");

        b.move_word_right();

        assert_eq!(b.cursor_col, 3);

        b.move_word_right();

        assert_eq!(b.cursor_col, 7);

        b.move_word_right();

        assert_eq!(b.cursor_col, 12);

        b.move_word_left();

        assert_eq!(b.cursor_col, 9);
    }

    #[test]
    fn delete_word_left_removes_token() {
        let mut b = buf("foo bar");

        b.cursor_col = 7;

        b.delete_word_left();

        assert_eq!(b.rope.to_string(), "foo ");

        assert_eq!(b.cursor_col, 4);
    }

    #[test]
    fn indent_and_outdent_line() {
        let mut b = buf("foo");

        b.cursor_col = 1;

        b.indent_line();

        assert_eq!(b.rope.to_string(), "    foo");

        assert_eq!(b.cursor_col, 5);

        b.outdent_line();

        assert_eq!(b.rope.to_string(), "foo");

        assert_eq!(b.cursor_col, 1);

        b.outdent_line();

        assert_eq!(b.rope.to_string(), "foo");
    }

    #[test]
    fn outdent_handles_tab_and_partial() {
        let mut b = buf("\tx");

        b.outdent_line();

        assert_eq!(b.rope.to_string(), "x");

        let mut b2 = buf("  y");

        b2.cursor_col = 2;

        b2.outdent_line();

        assert_eq!(b2.rope.to_string(), "y");

        assert_eq!(b2.cursor_col, 0);
    }

    #[test]
    fn doc_start_end() {
        let mut b = buf("ab\ncd\nef");

        b.move_doc_end();

        assert_eq!(b.cursor_line, 2);

        assert_eq!(b.cursor_col, 2);

        b.move_doc_start();

        assert_eq!(b.cursor_line, 0);

        assert_eq!(b.cursor_col, 0);
    }

    #[test]
    fn modified_clears_when_undone_to_saved() {
        let mut b = buf("hello");

        b.cursor_col = 5;

        b.insert_char('!');

        assert!(b.modified, "an edit marks the buffer modified");

        b.undo();

        assert!(!b.modified, "undoing back to the saved text clears modified");
    }

    #[test]
    fn modified_clears_when_edit_is_reverted_by_hand() {
        let mut b = buf("ab");

        b.cursor_col = 2;

        b.insert_char('c');

        assert!(b.modified);

        b.backspace();

        assert!(!b.modified, "deleting back to the saved text clears modified");
    }

    #[test]
    fn save_makes_current_text_the_new_clean_state() {
        let mut b = buf("ab");

        b.cursor_col = 2;

        b.insert_char('c');

        // Without touching the filesystem, mimic a save by resyncing `saved`.
        b.saved = b.rope.clone();

        b.refresh_modified();

        assert!(!b.modified);

        b.insert_char('d');

        assert!(b.modified);
    }

    #[test]
    fn undo_redo_coalesces_typing() {
        let mut b = buf("");

        for c in "abc".chars() {
            b.insert_char(c);
        }

        assert_eq!(b.rope.to_string(), "abc");

        b.undo();

        assert_eq!(b.rope.to_string(), "", "one undo should drop the whole typing burst");

        b.redo();

        assert_eq!(b.rope.to_string(), "abc");
    }

    #[test]
    fn undo_is_word_granular() {
        let mut b = buf("");

        for c in "foo bar baz".chars() {
            b.insert_char(c);
        }

        b.undo();

        assert_eq!(b.rope.to_string(), "foo bar ");

        b.undo();

        assert_eq!(b.rope.to_string(), "foo ");

        b.undo();

        assert_eq!(b.rope.to_string(), "");
    }

    #[test]
    fn undo_breaks_on_cursor_jump() {
        let mut b = buf("");

        b.insert_char('a');

        b.insert_char('b');

        b.move_to_char(0);

        b.insert_char('x');

        assert_eq!(b.rope.to_string(), "xab");

        b.undo();

        assert_eq!(b.rope.to_string(), "ab");

        b.undo();

        assert_eq!(b.rope.to_string(), "");
    }

    #[test]
    fn undo_separates_typing_from_newline() {
        let mut b = buf("");

        b.insert_char('a');

        b.insert_newline();

        b.insert_char('b');

        b.undo();

        assert_eq!(b.rope.to_string(), "a\n");

        b.undo();

        assert_eq!(b.rope.to_string(), "a");
    }

    #[test]
    fn selection_copy_delete_and_replace() {
        let mut b = buf("hello world");

        b.cursor_col = 0;

        b.sel(true);

        for _ in 0..5 {
            b.move_right();
        }

        assert_eq!(b.selected_text().as_deref(), Some("hello"));

        // Typing over the selection replaces it.
        b.insert_char('H');

        assert_eq!(b.rope.to_string(), "H world");

        assert!(b.selection().is_none());
    }

    #[test]
    fn para_motion_jumps_blank_lines() {
        let mut b = buf("a\nb\n\nc\nd\n");

        b.move_para_down();

        assert_eq!(b.cursor_line, 2); // the blank separator line

        b.move_para_down();

        assert_eq!(b.cursor_line, 5); // trailing blank line after the last block

        b.move_para_up();

        assert_eq!(b.cursor_line, 2);
    }

    #[test]
    fn search_finds_and_wraps() {
        let r = Rope::from_str("alpha beta alpha");

        assert_eq!(find_next(&r, "alpha", 0), Some(0));

        assert_eq!(find_next(&r, "alpha", 1), Some(11));

        assert_eq!(find_next(&r, "alpha", 12), Some(0));

        assert_eq!(find_next(&r, "zzz", 0), None);
    }
}
