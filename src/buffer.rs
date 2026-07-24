use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use ropey::Rope;

use crate::highlight::HlCache;

const INDENT: &str = "    ";
const UNDO_LIMIT: usize = 600;

/// Result of checking the open file against its on-disk version.
#[derive(PartialEq, Clone, Copy, Debug)]
pub enum DiskEvent {
    /// No change, or the change was reconciled with no user action needed.
    Unchanged,

    /// The buffer was clean, so it was auto-reloaded from disk (undoable).
    Reloaded,

    /// The buffer has unsaved edits and the disk changed — needs the user.
    Conflict,
}

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

    /// Selection anchor at the time of the edit, so undo brings the selection
    /// back (e.g. deleting a selection then undoing re-selects it) rather than
    /// just reverting the text.
    anchor: Option<usize>,
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

    /// Modification time of the file as we last read/wrote it; used to notice
    /// edits made by another program. `None` for a not-yet-existing file.
    disk_mtime: Option<SystemTime>,

    /// `true` when the file changed on disk while this buffer had unsaved edits
    /// (a conflict the user must resolve with reload or overwrite).
    pub disk_changed: bool,
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

        let disk_mtime = file_mtime(&path);

        Ok(Self {
            saved: rope.clone(),
            rope,
            disk_mtime,
            disk_changed: false,
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

    /// Select the entire buffer (anchor at the start, cursor at the end).
    pub fn select_all(&mut self) {
        self.anchor = Some(0);

        self.move_to_char(self.rope.len_chars());
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

    /// Remove the whole line the caret sits on, closing the gap so the line
    /// below moves up. On the last line the preceding break is taken instead,
    /// otherwise deleting would leave a blank line behind.
    pub fn delete_line(&mut self) {
        let line = self.cursor_line;

        let (start, end) = if line < self.last_line() {
            (self.rope.line_to_char(line), self.rope.line_to_char(line + 1))
        } else {
            let mut start = self.rope.line_to_char(line);

            if start > 0 && self.rope.char(start - 1) == '\n' {
                start -= 1;

                if start > 0 && self.rope.char(start - 1) == '\r' {
                    start -= 1;
                }
            }

            (start, self.rope.len_chars())
        };

        if start == end {
            return;
        }

        self.record(EditKind::Other, start, true);

        self.rope.remove(start..end);

        self.anchor = None;

        self.move_to_char(start.min(self.rope.len_chars()));

        self.hl.invalidate(line.min(self.last_line()));

        self.refresh_modified();

        self.mark_edit(None);
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
            anchor: self.anchor,
        }
    }

    fn restore(&mut self, snap: Snapshot) {
        self.rope = snap.rope;

        self.cursor_line = snap.cursor_line.min(self.last_line());

        self.cursor_col = snap.cursor_col.min(self.line_len(self.cursor_line));

        self.desired_col = self.cursor_col;

        // Always set the anchor from the snapshot (clamped), so it can never
        // outlive the text it pointed into and leave a stale, misaligned
        // selection behind.
        self.anchor = snap.anchor.map(|a| a.min(self.rope.len_chars()));

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

        self.disk_changed = false;

        // Remember our own write so the next poll doesn't flag it as external.
        self.disk_mtime = file_mtime(&self.path);

        Ok(())
    }

    /// Compare the open file with its on-disk version. A clean buffer is
    /// auto-reloaded; a dirty one flags a conflict for the user to resolve.
    /// Cheap: only re-reads when the file's mtime actually changed.
    pub fn poll_disk(&mut self) -> DiskEvent {
        let Some(mtime) = file_mtime(&self.path) else {
            return DiskEvent::Unchanged;
        };

        if Some(mtime) == self.disk_mtime {
            return DiskEvent::Unchanged;
        }

        self.disk_mtime = Some(mtime);

        let Ok(text) = fs::read_to_string(&self.path) else {
            return DiskEvent::Unchanged;
        };

        let disk = Rope::from_str(&text);

        if disk == self.saved {
            return DiskEvent::Unchanged;
        }

        if disk == self.rope {
            // Disk now matches our buffer exactly — we are effectively saved.
            self.saved = disk;

            self.modified = false;

            self.disk_changed = false;

            return DiskEvent::Unchanged;
        }

        if self.rope == self.saved {
            self.apply_disk_content(disk);

            DiskEvent::Reloaded
        } else {
            self.disk_changed = true;

            DiskEvent::Conflict
        }
    }

    /// Replace the buffer with the file's current contents (used by Ctrl+R and
    /// conflict resolution). Undoable — Ctrl+Z restores the previous text.
    pub fn reload_from_disk(&mut self) -> Result<()> {
        let text = fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;

        self.apply_disk_content(Rope::from_str(&text));

        self.disk_mtime = file_mtime(&self.path);

        Ok(())
    }

    /// Swap in new content as one undoable step (push the current state first),
    /// then reset selection, clamp the cursor and refresh derived state.
    fn apply_disk_content(&mut self, disk: Rope) {
        self.undo_stack.push(self.snapshot());

        if self.undo_stack.len() > UNDO_LIMIT {
            self.undo_stack.remove(0);
        }

        self.redo_stack.clear();

        self.last_edit = None;

        self.last_edit_pos = None;

        self.last_insert_char = None;

        self.rope = disk.clone();

        self.saved = disk;

        self.anchor = None;

        self.clamp_cursor();

        self.hl.invalidate(0);

        self.modified = false;

        self.disk_changed = false;
    }

    fn clamp_cursor(&mut self) {
        self.cursor_line = self.cursor_line.min(self.last_line());

        self.cursor_col = self.cursor_col.min(self.line_len(self.cursor_line));

        self.desired_col = self.cursor_col;
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
    /// Lines the selection touches, or `None` when nothing is selected. A
    /// selection ending exactly at a line start does not pull that line in,
    /// otherwise selecting whole lines would indent one line too many.
    fn selected_line_span(&self) -> Option<(usize, usize)> {
        let (start, end) = self.selection()?;

        let first = self.rope.char_to_line(start);

        let mut last = self.rope.char_to_line(end);

        if last > first && end == self.rope.line_to_char(last) {
            last -= 1;
        }

        Some((first, last))
    }

    /// Width of one indent level at the head of `line`: a tab, or up to four
    /// spaces.
    fn leading_indent_width(&self, line: usize) -> usize {
        let start = self.rope.line_to_char(line);

        let len = self.line_len(line);

        if len == 0 {
            return 0;
        }

        if self.rope.char(start) == '\t' {
            return 1;
        }

        let mut n = 0;

        while n < INDENT.len() && n < len && self.rope.char(start + n) == ' ' {
            n += 1;
        }

        n
    }

    /// Indent every line the selection covers, or the caret line when nothing
    /// is selected.
    pub fn indent(&mut self) {
        match self.selected_line_span() {
            Some((first, last)) => self.shift_lines(first, last, true),

            None => self.indent_line(),
        }
    }

    /// Outdent every line the selection covers, or the caret line when nothing
    /// is selected.
    pub fn outdent(&mut self) {
        match self.selected_line_span() {
            Some((first, last)) => self.shift_lines(first, last, false),

            None => self.outdent_line(),
        }
    }

    /// Shift a run of lines by one indent level as a single undo step, keeping
    /// the selection over the same text so the key can be pressed repeatedly.
    fn shift_lines(&mut self, first: usize, last: usize, indent: bool) {
        // Measure first: outdenting lines that have no indent left must not
        // push a no-op onto the undo stack.
        let widths: Vec<usize> = if indent {
            vec![INDENT.chars().count(); last - first + 1]
        } else {
            (first..=last).map(|l| self.leading_indent_width(l)).collect()
        };

        if widths.iter().all(|w| *w == 0) {
            return;
        }

        let anchor_pos = self.anchor.map(|a| {
            let line = self.rope.char_to_line(a);

            (line, a - self.rope.line_to_char(line))
        });

        let cursor_pos = (self.cursor_line, self.cursor_col);

        self.record(EditKind::Other, self.rope.line_to_char(first), true);

        for (offset, width) in widths.iter().enumerate() {
            if *width == 0 {
                continue;
            }

            let start = self.rope.line_to_char(first + offset);

            if indent {
                self.rope.insert(start, INDENT);
            } else {
                self.rope.remove(start..start + width);
            }
        }

        // Columns on the touched lines moved by exactly what was added or cut.
        let moved = |(line, col): (usize, usize)| -> (usize, usize) {
            if line < first || line > last {
                return (line, col);
            }

            let width = widths[line - first] as isize;

            let delta = if indent { width } else { -width };

            (line, (col as isize + delta).max(0) as usize)
        };

        if let Some(pos) = anchor_pos {
            let (line, col) = moved(pos);

            self.anchor = Some(self.rope.line_to_char(line) + col.min(self.line_len(line)));
        }

        let (line, col) = moved(cursor_pos);

        self.cursor_line = line;

        self.cursor_col = col.min(self.line_len(line));

        self.desired_col = self.cursor_col;

        self.hl.invalidate(first);

        self.refresh_modified();

        self.mark_edit(None);
    }

    /// Toggle comments on the caret line, or every line the selection covers.
    /// Line-comment languages toggle per line at the block's minimum indent;
    /// block-only languages (CSS, HTML) wrap the covered lines instead. A
    /// language with no known token does nothing.
    pub fn toggle_comment(&mut self) {
        let style = crate::comment::for_syntax(self.hl.syntax_name());

        let had_selection = self.selection().is_some();

        let (first, last) = self.selected_line_span().unwrap_or((self.cursor_line, self.cursor_line));

        let changed = if let Some(token) = style.line {
            self.toggle_line_comment(first, last, token)
        } else if let Some((open, close)) = style.block {
            self.toggle_block_comment(first, last, open, close)
        } else {
            false
        };

        if !changed {
            return;
        }

        // Toggling never adds or removes lines, so `first..=last` are still
        // valid. Reselect the whole run when there was a selection so the key
        // repeats; otherwise keep the caret on its line.
        if had_selection {
            self.anchor = Some(self.rope.line_to_char(first));

            let end = self.rope.line_to_char(last) + self.line_len(last);

            self.move_to_char(end);

            self.anchor = Some(self.rope.line_to_char(first));
        } else {
            let col = self.cursor_col.min(self.line_len(self.cursor_line));

            self.cursor_col = col;

            self.desired_col = col;
        }

        self.hl.invalidate(first);

        self.refresh_modified();

        self.mark_edit(None);
    }

    /// True when the line, ignoring leading whitespace, already begins with the
    /// comment token. Blank lines are treated as already commented so a block
    /// with blank lines still uncomments in one press.
    fn line_is_commented(&self, line: usize, token: &str) -> bool {
        let text: String = self.rope.line(line).chars().take_while(|c| *c != '\n').collect();

        let trimmed = text.trim_start();

        trimmed.is_empty() || trimmed.starts_with(token)
    }

    fn toggle_line_comment(&mut self, first: usize, last: usize, token: &str) -> bool {
        let non_blank: Vec<usize> = (first..=last)
            .filter(|&l| self.leading_whitespace_cols(l) < self.line_len(l))
            .collect();

        if non_blank.is_empty() {
            return false;
        }

        // Uncomment only when every non-blank line is already commented,
        // matching the usual editor toggle.
        let commenting = !non_blank.iter().all(|&l| self.line_is_commented(l, token));

        self.record(EditKind::Other, self.rope.line_to_char(first), true);

        if commenting {
            // Insert at the shallowest indent so relative indentation is kept.
            let col = non_blank
                .iter()
                .map(|&l| self.leading_whitespace_cols(l))
                .min()
                .unwrap_or(0);

            let insert = format!("{token} ");

            for &l in non_blank.iter().rev() {
                let at = self.rope.line_to_char(l) + col;

                self.rope.insert(at, &insert);
            }
        } else {
            for &l in non_blank.iter().rev() {
                self.remove_line_token(l, token);
            }
        }

        true
    }

    /// Leading-whitespace width of a line in chars.
    fn leading_whitespace_cols(&self, line: usize) -> usize {
        self.rope
            .line(line)
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .count()
    }

    /// Strip the comment token (and one following space, if the toggle added
    /// one) from a commented line.
    fn remove_line_token(&mut self, line: usize, token: &str) {
        let start = self.rope.line_to_char(line);

        let indent = self.leading_whitespace_cols(line);

        let after = start + indent;

        let tail: String = self.rope.line(line).chars().skip(indent).collect();

        if !tail.starts_with(token) {
            return;
        }

        let mut remove = token.chars().count();

        if tail[token.len()..].starts_with(' ') {
            remove += 1;
        }

        self.rope.remove(after..after + remove);
    }

    fn toggle_block_comment(&mut self, first: usize, last: usize, open: &str, close: &str) -> bool {
        let start = self.rope.line_to_char(first) + self.leading_whitespace_cols(first);

        let end = self.rope.line_to_char(last) + self.line_len(last);

        if start >= end {
            return false;
        }

        let inner: String = self.rope.slice(start..end).to_string();

        let trimmed = inner.trim();

        self.record(EditKind::Other, start, true);

        if trimmed.starts_with(open) && trimmed.ends_with(close) && trimmed.len() >= open.len() + close.len() {
            // Unwrap: rebuild the span without the delimiters and the padding
            // the wrap added.
            let body = trimmed[open.len()..trimmed.len() - close.len()].trim().to_string();

            self.rope.remove(start..end);

            self.rope.insert(start, &body);
        } else {
            self.rope.insert(end, &format!(" {close}"));

            self.rope.insert(start, &format!("{open} "));
        }

        true
    }

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

        let remove = self.leading_indent_width(self.cursor_line);

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

    /// Move the cursor to a line and column, clamping both into the buffer
    /// (used by mouse clicks, where either can point past the text).
    pub fn move_to_pos(&mut self, line: usize, col: usize) {
        let line = line.min(self.last_line());

        let col = col.min(self.line_len(line));

        self.move_to_char(self.rope.line_to_char(line) + col);
    }

    /// Scroll the viewport by `delta` lines, leaving the caret where it is.
    pub fn scroll_view(&mut self, delta: isize) {
        let last = self.last_line() as isize;

        self.scroll_row = (self.scroll_row as isize + delta).clamp(0, last) as usize;
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

    /// Delete from the cursor to the end of the word ahead (forward word delete).
    pub fn delete_word_right(&mut self) {
        let start = self.cursor_idx();

        let end = self.word_right_from(start);

        if end > start {
            let line = self.rope.char_to_line(start);

            self.record(EditKind::Other, start, true);

            self.rope.remove(start..end);

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
/// Char-column ranges of every non-overlapping `needle` in `text`, for painting
/// search matches on a rendered line. Returns nothing for an empty needle: a
/// zero-width match would never advance the scan.
pub fn match_columns(text: &str, needle: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();

    if needle.is_empty() {
        return out;
    }

    let width = needle.chars().count();

    let mut byte = 0;

    let mut col = 0;

    // Walk forward keeping a running char count, so a line with many matches
    // costs one pass rather than one re-count per match.
    while let Some(rel) = text[byte..].find(needle) {
        let start = byte + rel;

        col += text[byte..start].chars().count();

        out.push((col, col + width));

        col += width;

        byte = start + needle.len();
    }

    out
}

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

fn file_mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
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
            disk_mtime: None,
            disk_changed: false,
        }
    }

    fn buf_lang(text: &str, syntax: &str) -> Buffer {
        let mut b = buf(text);

        b.hl = HlCache::new(syntax.to_string());

        b
    }

    fn select_lines(b: &mut Buffer, first: usize, last: usize) {
        b.anchor = Some(b.rope.line_to_char(first));

        let end = b.rope.line_to_char(last) + b.line_len(last);

        b.move_to_char(end);
    }

    #[test]
    fn comment_toggles_a_selection_and_undoes_as_one() {
        let src = "fn f() {\n    let x = 1;\n    let y = 2;\n}\n";

        let mut b = buf_lang(src, "Rust");

        select_lines(&mut b, 1, 2);

        b.toggle_comment();

        assert_eq!(b.rope.to_string(), "fn f() {\n    // let x = 1;\n    // let y = 2;\n}\n");

        assert!(b.selection().is_some(), "selection survives so the key repeats");

        b.toggle_comment();

        assert_eq!(b.rope.to_string(), src, "second press uncomments");

        b.undo();

        assert_eq!(
            b.rope.to_string(),
            "fn f() {\n    // let x = 1;\n    // let y = 2;\n}\n",
            "each toggle is a single undo step"
        );
    }

    #[test]
    fn comment_inserts_at_the_shallowest_indent() {
        let mut b = buf_lang("    deep\n  shallow\n", "Rust");

        select_lines(&mut b, 0, 1);

        b.toggle_comment();

        // Token goes in at column 2 (the shallower line), so the deeper line
        // keeps its extra indent relative to it.
        assert_eq!(b.rope.to_string(), "  //   deep\n  // shallow\n");
    }

    #[test]
    fn comment_skips_blank_lines() {
        let mut b = buf_lang("a\n\nb\n", "Rust");

        select_lines(&mut b, 0, 2);

        b.toggle_comment();

        assert_eq!(b.rope.to_string(), "// a\n\n// b\n", "the blank line is left alone");
    }

    #[test]
    fn comment_on_a_mix_comments_all() {
        let mut b = buf_lang("// a\nb\n", "Rust");

        select_lines(&mut b, 0, 1);

        b.toggle_comment();

        assert_eq!(b.rope.to_string(), "// // a\n// b\n", "a mixed block comments every line");
    }

    #[test]
    fn comment_single_line_without_selection() {
        let mut b = buf_lang("x = 1\n", "Python");

        b.toggle_comment();

        assert_eq!(b.rope.to_string(), "# x = 1\n");

        assert!(b.selection().is_none(), "no selection is created");

        b.toggle_comment();

        assert_eq!(b.rope.to_string(), "x = 1\n");
    }

    #[test]
    fn block_comment_wraps_and_unwraps() {
        let mut b = buf_lang("a { color: red }\n", "CSS");

        b.toggle_comment();

        assert_eq!(b.rope.to_string(), "/* a { color: red } */\n");

        b.toggle_comment();

        assert_eq!(b.rope.to_string(), "a { color: red }\n");
    }

    #[test]
    fn comment_does_nothing_for_a_language_with_no_token() {
        let mut b = buf_lang("hello\n", "Plain Text");

        b.toggle_comment();

        assert_eq!(b.rope.to_string(), "hello\n");

        assert!(!b.modified, "an unknown language leaves the buffer clean");
    }

    #[test]
    fn match_columns_finds_every_hit_without_overlapping() {
        assert_eq!(match_columns("a foo b foo", "foo"), vec![(2, 5), (8, 11)]);

        // Non-overlapping: "aa" in "aaaa" is two matches, not three.
        assert_eq!(match_columns("aaaa", "aa"), vec![(0, 2), (2, 4)]);

        assert_eq!(match_columns("nothing here", "zzz"), vec![]);
    }

    /// An empty needle would match at every position without ever advancing the
    /// scan, so it must yield nothing rather than spin.
    #[test]
    fn match_columns_ignores_an_empty_needle() {
        assert_eq!(match_columns("anything", ""), vec![]);
    }

    /// Columns are counted in chars, matching how the renderer places cells, so
    /// multi-byte text ahead of a match must not shift the highlight.
    #[test]
    fn match_columns_counts_chars_not_bytes() {
        assert_eq!(match_columns("héllo wörld hi", "hi"), vec![(12, 14)]);

        assert_eq!(match_columns("日本語 test", "test"), vec![(4, 8)]);
    }

    #[test]
    fn undo_after_deleting_a_selection_brings_the_selection_back() {
        let mut b = buf("hello world\n");

        b.anchor = Some(0);

        b.move_to_char(5); // select "hello"

        assert_eq!(b.selected_text().as_deref(), Some("hello"));

        b.delete_selection();

        assert_eq!(b.rope.to_string(), " world\n");

        assert!(b.selection().is_none());

        b.undo();

        assert_eq!(b.rope.to_string(), "hello world\n", "the text comes back");

        assert_eq!(b.selected_text().as_deref(), Some("hello"), "and so does the selection");

        b.redo();

        assert_eq!(b.rope.to_string(), " world\n", "redo deletes it again");

        assert!(b.selection().is_none());
    }

    /// Undo must set the anchor from the snapshot, never leave the one that was
    /// live at undo time, which would point into different text and show a
    /// selection that was never part of the edit.
    #[test]
    fn undo_does_not_leave_a_stale_selection() {
        let mut b = buf("abcdef\n");

        b.insert_char('X'); // one edit; its snapshot had no selection

        // A selection made after the edit, unrelated to it.
        b.anchor = Some(3);

        b.cursor_col = 5;

        b.undo();

        assert_eq!(b.rope.to_string(), "abcdef\n");

        assert!(
            b.selection().is_none(),
            "the stale anchor is cleared, not applied to the reverted text"
        );

        // The formerly out-of-range case must not panic when read.
        let _ = b.selected_text();
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
    fn delete_word_right_removes_word_ahead() {
        let mut b = buf("foo bar");

        b.delete_word_right(); // cursor at 0 → removes "foo"

        assert_eq!(b.rope.to_string(), " bar");

        assert_eq!(b.cursor_col, 0);
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
    fn poll_auto_reloads_a_clean_buffer() {
        let path = std::env::temp_dir().join("opencode_poll_clean.txt");

        fs::write(&path, "v1\n").unwrap();

        let mut b = Buffer::open(path.clone(), "Plain Text".to_string()).unwrap();

        fs::write(&path, "v2\n").unwrap(); // external change

        b.disk_mtime = None; // force re-detect regardless of mtime granularity

        assert_eq!(b.poll_disk(), DiskEvent::Reloaded);

        assert_eq!(b.rope.to_string(), "v2\n");

        assert!(!b.modified);

        b.undo(); // reload is undoable

        assert_eq!(b.rope.to_string(), "v1\n");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn poll_flags_conflict_for_a_dirty_buffer() {
        let path = std::env::temp_dir().join("opencode_poll_conflict.txt");

        fs::write(&path, "v1\n").unwrap();

        let mut b = Buffer::open(path.clone(), "Plain Text".to_string()).unwrap();

        b.insert_char('X'); // dirty: "Xv1\n"

        fs::write(&path, "v2\n").unwrap();

        b.disk_mtime = None;

        assert_eq!(b.poll_disk(), DiskEvent::Conflict);

        assert!(b.disk_changed);

        assert_eq!(b.rope.to_string(), "Xv1\n"); // not auto-reloaded

        b.reload_from_disk().unwrap(); // Ctrl+R resolves it

        assert_eq!(b.rope.to_string(), "v2\n");

        assert!(!b.disk_changed);

        b.undo(); // reload is undoable -> our edits come back

        assert_eq!(b.rope.to_string(), "Xv1\n");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn poll_ignores_our_own_save() {
        let path = std::env::temp_dir().join("opencode_poll_save.txt");

        fs::write(&path, "v1\n").unwrap();

        let mut b = Buffer::open(path.clone(), "Plain Text".to_string()).unwrap();

        b.insert_char('X');

        b.save().unwrap();

        b.disk_mtime = None;

        assert_eq!(b.poll_disk(), DiskEvent::Unchanged);

        assert!(!b.disk_changed);

        let _ = fs::remove_file(path);
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
