//! Lean's language server, for the errors of the open Lean file. It starts only
//! when a Lean file is open and Lean is installed, and stops when that file is
//! closed, so every other file costs nothing.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ropey::Rope;
use serde_json::{Value, json};

use crate::lsp::{Server, Stopped};

/// How long after an edit results are still expected. The server reports that
/// it is checking within milliseconds, but until it does there is nothing
/// else that says to keep watching.
const ANSWER_WINDOW: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,

    Warning,

    Information,

    Hint,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    /// Char index in the buffer where it starts.
    pub start: usize,

    /// Char index just past its end.
    pub end: usize,

    pub severity: Severity,

    pub message: String,
}

/// Why there is no session for a Lean file.
#[derive(Debug)]
pub enum Unavailable {
    /// Neither `lake` nor `lean` is on PATH or in `~/.elan/bin`.
    NotInstalled,

    Spawn(std::io::Error),
}

pub struct Session {
    server: Server,

    root: PathBuf,

    /// The id of the `initialize` request until it is answered.
    initializing: Option<i64>,

    doc: Option<Doc>,

    /// When the server was last handed new text.
    last_sent: Option<Instant>,
}

struct Doc {
    path: PathBuf,

    uri: String,

    version: i64,

    /// The text as the server has it, to diff the next edit against.
    synced: Rope,

    /// `didOpen` waits for the server to finish initializing.
    opened: bool,

    diagnostics: Vec<Diagnostic>,

    /// Lines still being checked, as `[first, last]` pairs.
    processing: Vec<(usize, usize)>,
}

impl Session {
    /// Start the server for the project `file` belongs to: `lake serve` in a
    /// Lake project, so imports from it and its dependencies resolve, or a
    /// bare `lean --server` for a file on its own.
    pub fn start(file: &Path) -> Result<Self, Unavailable> {
        let (root, lake) = project_of(file);

        let (program, args): (&str, &[&str]) = if lake { ("lake", &["serve"]) } else { ("lean", &["--server"]) };

        let program = find_program(program).ok_or(Unavailable::NotInstalled)?;

        let mut server = Server::spawn(&program, args, &root).map_err(Unavailable::Spawn)?;

        let id = server.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": uri_of(&root),
                "clientInfo": { "name": "ocode", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": {
                    "general": { "positionEncodings": ["utf-16"] },
                    "textDocument": { "publishDiagnostics": {} },
                },
            }),
        );

        Ok(Self { server, root, initializing: Some(id), doc: None, last_sent: None })
    }

    /// The project the server was started for.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file the server is checking.
    pub fn path(&self) -> Option<&Path> {
        self.doc.as_ref().map(|doc| doc.path.as_path())
    }

    /// Hand the server `path` with its text, closing whatever was open before.
    pub fn open(&mut self, path: &Path, text: &Rope) {
        self.close();

        self.doc = Some(Doc {
            path: path.to_path_buf(),
            uri: uri_of(&canonical(path)),
            version: 1,
            synced: text.clone(),
            opened: false,
            diagnostics: Vec::new(),
            processing: Vec::new(),
        });

        self.send_open();
    }

    fn send_open(&mut self) {
        if self.initializing.is_some() {
            return;
        }

        let Some(doc) = self.doc.as_mut().filter(|doc| !doc.opened) else {
            return;
        };

        self.server.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": doc.uri,
                    "languageId": "lean4",
                    "version": doc.version,
                    "text": doc.synced.to_string(),
                },
            }),
        );

        doc.opened = true;

        self.last_sent = Some(Instant::now());
    }

    pub fn close(&mut self) {
        if let Some(doc) = self.doc.take().filter(|doc| doc.opened) {
            self.server.notify("textDocument/didClose", json!({ "textDocument": { "uri": doc.uri } }));
        }
    }

    /// Tell the server about edits since the last sync, as the one change that
    /// covers them all.
    pub fn sync(&mut self, text: &Rope) {
        let Some(doc) = self.doc.as_mut() else {
            return;
        };

        let Some((range, inserted)) = edit_between(&doc.synced, text) else {
            return;
        };

        if doc.opened {
            let (start_line, start_col) = position(&doc.synced, range.start);

            let (end_line, end_col) = position(&doc.synced, range.end);

            doc.version += 1;

            self.server.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": doc.uri, "version": doc.version },
                    "contentChanges": [{
                        "range": {
                            "start": { "line": start_line, "character": start_col },
                            "end": { "line": end_line, "character": end_col },
                        },
                        "text": inserted,
                    }],
                }),
            );

            self.last_sent = Some(Instant::now());
        }

        // Until the server reports on the new text, keep what it said about
        // the old in place over the same code, the way an editor carries its
        // marks along with an edit.
        let inserted_len = inserted.chars().count();

        for d in &mut doc.diagnostics {
            d.start = carried(d.start, &range, inserted_len);

            d.end = carried(d.end, &range, inserted_len).max(d.start);
        }

        // Before the open is sent, the open itself will carry the new text.
        doc.synced = text.clone();
    }

    /// Take in what the server has sent. Returns `true` when anything that is
    /// drawn changed, and `Err` once the server is gone.
    pub fn pump(&mut self) -> Result<bool, Stopped> {
        let mut changed = false;

        for message in self.server.poll()? {
            changed |= self.handle(message);
        }

        Ok(changed)
    }

    fn handle(&mut self, message: Value) -> bool {
        let id = message.get("id").cloned();

        match (id, message.get("method").and_then(Value::as_str)) {
            // The server asks things of the client too (registering a file
            // watcher, refreshing inlay hints). Nothing here needs more than an
            // acknowledgement, and an unanswered request can hold it up.
            (Some(id), Some(_)) => {
                self.server.respond(id, Value::Null);

                false
            }

            (Some(id), None) => {
                if id.as_i64().is_some() && id.as_i64() == self.initializing {
                    self.initializing = None;

                    self.server.notify("initialized", json!({}));

                    self.send_open();
                }

                false
            }

            (None, Some("textDocument/publishDiagnostics")) => self.take_diagnostics(&message["params"]),

            (None, Some("$/lean/fileProgress")) => self.take_progress(&message["params"]),

            _ => false,
        }
    }

    /// The document the params are about, if it is ours and current. A stale
    /// version is dropped: its positions refer to text that has since changed,
    /// and the current version's report follows.
    fn current_doc(&mut self, params: &Value, uri: &Value) -> Option<&mut Doc> {
        let doc = self.doc.as_mut()?;

        let ours = uri.as_str().is_some_and(|uri| same_uri(uri, &doc.uri));

        let version = params.get("version").or_else(|| params["textDocument"].get("version"));

        let current = version.and_then(Value::as_i64).is_none_or(|v| v == doc.version);

        (ours && current).then_some(doc)
    }

    fn take_diagnostics(&mut self, params: &Value) -> bool {
        let Some(doc) = self.current_doc(params, &params["uri"]) else {
            return false;
        };

        let diagnostics: Vec<Diagnostic> = params["diagnostics"]
            .as_array()
            .map(|list| list.iter().filter_map(|d| diagnostic(&doc.synced, d)).collect())
            .unwrap_or_default();

        if diagnostics == doc.diagnostics {
            return false;
        }

        doc.diagnostics = diagnostics;

        true
    }

    fn take_progress(&mut self, params: &Value) -> bool {
        let Some(doc) = self.current_doc(params, &params["textDocument"]["uri"]) else {
            return false;
        };

        let processing: Vec<(usize, usize)> = params["processing"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|p| {
                        let start = p["range"]["start"]["line"].as_u64()? as usize;

                        let end = p["range"]["end"]["line"].as_u64()? as usize;

                        Some((start, end.max(start)))
                    })
                    .collect()
            })
            .unwrap_or_default();

        if processing == doc.processing {
            return false;
        }

        doc.processing = processing;

        true
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        self.doc.as_ref().map(|doc| doc.diagnostics.as_slice()).unwrap_or_default()
    }

    /// Whether `line` is still being checked.
    pub fn is_processing(&self, line: usize) -> bool {
        self.doc
            .as_ref()
            .is_some_and(|doc| doc.processing.iter().any(|(first, last)| (*first..=*last).contains(&line)))
    }

    /// Starting up, checking, or just handed an edit: worth waking up for,
    /// since results are coming.
    pub fn busy(&self) -> bool {
        self.initializing.is_some()
            || self.doc.as_ref().is_some_and(|doc| !doc.processing.is_empty())
            || self.last_sent.is_some_and(|at| at.elapsed() < ANSWER_WINDOW)
    }

    /// Still waiting for the server to come up.
    pub fn starting(&self) -> bool {
        self.initializing.is_some()
    }

    /// Whether any line is still being checked.
    pub fn checking(&self) -> bool {
        self.doc.as_ref().is_some_and(|doc| !doc.processing.is_empty())
    }

    /// The server's last words, to say why it stopped or what it is doing.
    pub fn last_log(&self) -> Option<String> {
        self.server.last_log()
    }

    pub fn stop(mut self) {
        self.close();

        self.server.stop();
    }
}

/// Where a position ends up after `replaced` is swapped for `inserted_len`
/// chars: unmoved before it, shifted after it, and at the end of the new
/// text if it was inside what was replaced.
fn carried(at: usize, replaced: &Range<usize>, inserted_len: usize) -> usize {
    if at <= replaced.start {
        at
    } else if at >= replaced.end {
        at - replaced.len() + inserted_len
    } else {
        replaced.start + inserted_len
    }
}

fn diagnostic(text: &Rope, raw: &Value) -> Option<Diagnostic> {
    let range = &raw["range"];

    let start = char_index(text, range["start"]["line"].as_u64()? as usize, range["start"]["character"].as_u64()? as usize);

    let end = char_index(text, range["end"]["line"].as_u64()? as usize, range["end"]["character"].as_u64()? as usize);

    // LSP leaves severity optional; a message without one reads as an error.
    let severity = match raw["severity"].as_u64() {
        Some(2) => Severity::Warning,

        Some(3) => Severity::Information,

        Some(4) => Severity::Hint,

        _ => Severity::Error,
    };

    Some(Diagnostic {
        start,
        end: end.max(start),
        severity,
        message: raw["message"].as_str().unwrap_or_default().to_string(),
    })
}

/// The project a Lean file belongs to: the nearest folder with a Lake file,
/// else the nearest with a `lean-toolchain` (which picks the Lean version),
/// else the file's own folder. `true` when it is a Lake project.
fn project_of(file: &Path) -> (PathBuf, bool) {
    let file = canonical(file);

    let dir = file.parent().map(Path::to_path_buf).unwrap_or_default();

    if let Some(root) = dir
        .ancestors()
        .find(|d| d.join("lakefile.lean").is_file() || d.join("lakefile.toml").is_file())
    {
        return (root.to_path_buf(), true);
    }

    let root = dir.ancestors().find(|d| d.join("lean-toolchain").is_file()).unwrap_or(&dir);

    (root.to_path_buf(), false)
}

/// The real path behind any symlink, so the project root and the file's URI
/// spell the same folder the same way (on macOS `/tmp` is `/private/tmp`).
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The project root a server for `file` would start in, to tell whether a
/// running server can take it.
pub fn root_of(file: &Path) -> PathBuf {
    project_of(file).0
}

/// A program on PATH, or where elan installs it: a terminal that never sourced
/// the shell profile elan edited still finds Lean.
fn find_program(name: &str) -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).map(|dir| dir.join(name)).collect::<Vec<_>>())
        .unwrap_or_default();

    let from_elan = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".elan").join("bin").join(name));

    on_path.into_iter().chain(from_elan).find(|candidate| candidate.is_file())
}

/// A `file://` URI, percent-encoding everything but the unreserved characters
/// and the slashes, so a folder named `Toán học` survives the trip.
fn uri_of(path: &Path) -> String {
    let mut uri = String::from("file://");

    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }

    uri
}

/// Compare URIs the way the server might have re-encoded them.
fn same_uri(a: &str, b: &str) -> bool {
    decode(a) == decode(b)
}

fn decode(uri: &str) -> Vec<u8> {
    let bytes = uri.as_bytes();

    let mut out = Vec::with_capacity(bytes.len());

    let mut i = 0;

    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());

        match (bytes[i], hex.and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(byte)) => {
                out.push(byte);

                i += 3;
            }

            (byte, _) => {
                out.push(byte);

                i += 1;
            }
        }
    }

    out
}

/// The LSP position of a char index: its line, and its column counted in
/// UTF-16 code units, which is what the protocol means by a character. `𝔸`
/// is one char but two units.
fn position(text: &Rope, char_idx: usize) -> (usize, usize) {
    let line = text.char_to_line(char_idx);

    let line_start = text.line_to_char(line);

    (line, text.char_to_utf16_cu(char_idx) - text.char_to_utf16_cu(line_start))
}

/// The char index of an LSP position, kept inside its line.
fn char_index(text: &Rope, line: usize, utf16_col: usize) -> usize {
    if line >= text.len_lines() {
        return text.len_chars();
    }

    let line_start = text.line_to_char(line);

    let line_end = line_start + text.line(line).chars().take_while(|c| *c != '\n' && *c != '\r').count();

    let target = text.char_to_utf16_cu(line_start) + utf16_col;

    text.utf16_cu_to_char(target.min(text.char_to_utf16_cu(line_end)))
}

/// The one edit that turns `old` into `new`: the char range it replaces in
/// `old` and the text that goes there. Typing, a paste, an undo or a reload
/// all come out as a single change, found by trimming what both share at the
/// front and at the back.
fn edit_between(old: &Rope, new: &Rope) -> Option<(Range<usize>, String)> {
    if old == new {
        return None;
    }

    let prefix = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();

    let room = old.len_chars().min(new.len_chars()) - prefix;

    let suffix = old
        .chars_at(old.len_chars())
        .reversed()
        .zip(new.chars_at(new.len_chars()).reversed())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();

    let replaced = prefix..old.len_chars() - suffix;

    let inserted = new.slice(prefix..new.len_chars() - suffix).to_string();

    Some((replaced, inserted))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Apply an LSP change to text the way a server does, by line and UTF-16
    /// column, to check the change sent says what the edit did.
    fn apply(old: &Rope, change: &(Range<usize>, String)) -> String {
        let (start, end) = (position(old, change.0.start), position(old, change.0.end));

        let mut text = old.clone();

        let (from, to) = (char_index(old, start.0, start.1), char_index(old, end.0, end.1));

        text.remove(from..to);

        text.insert(from, &change.1);

        text.to_string()
    }

    #[test]
    fn one_change_covers_any_edit() {
        let cases = [
            ("theorem t : p := by\n  simp\n", "theorem t : p := by\n  simp only []\n"),
            ("a\nb\nc\n", "a\nc\n"),
            ("def 𝔸 := 1\n", "def 𝔸𝔹 := 1\n"),
            ("x", ""),
            ("", "∀ x, x = x\n"),
            ("aaaa", "aa"),
            ("abcabc", "abc"),
        ];

        for (before, after) in cases {
            let old = Rope::from_str(before);

            let change = edit_between(&old, &Rope::from_str(after)).expect("the texts differ");

            assert_eq!(apply(&old, &change), after, "{before:?} -> {after:?}");
        }

        assert_eq!(edit_between(&Rope::from_str("same"), &Rope::from_str("same")), None);
    }

    /// Marks follow the code they point at through an edit made before the
    /// server has looked at it again.
    #[test]
    fn diagnostics_move_with_an_edit() {
        let replaced = 10..12;

        assert_eq!(carried(4, &replaced, 5), 4, "before the edit, unmoved");

        assert_eq!(carried(10, &replaced, 5), 10, "at its start, unmoved");

        assert_eq!(carried(20, &replaced, 5), 23, "after it, shifted by the growth");

        assert_eq!(carried(11, &replaced, 5), 15, "inside it, to the end of the new text");

        assert_eq!(carried(20, &(10..15), 0), 15, "a deletion pulls later marks back");
    }

    #[test]
    fn positions_count_utf16_units() {
        let text = Rope::from_str("def 𝔸z : Nat := true\nα\n");

        // The real server put this error at character 17: `𝔸` counts twice.
        let start = text.to_string().find("true").map(|b| text.byte_to_char(b)).unwrap();

        assert_eq!(position(&text, start), (0, 17));

        assert_eq!(char_index(&text, 0, 17), start);

        // A column past the end of the line stays in the line.
        assert_eq!(char_index(&text, 1, 40), text.line_to_char(1) + 1);

        assert_eq!(char_index(&text, 9, 0), text.len_chars(), "a line past the end is the end");
    }

    #[test]
    fn diagnostics_read_like_the_server_sends_them() {
        let text = Rope::from_str("example : 1 = 2 := by rfl\ntheorem u : True := by sorry\n");

        let raw = json!({
            "range": { "start": { "line": 0, "character": 22 }, "end": { "line": 0, "character": 25 } },
            "severity": 1,
            "message": "Tactic `rfl` failed",
        });

        let d = diagnostic(&text, &raw).unwrap();

        assert_eq!(text.slice(d.start..d.end).to_string(), "rfl");

        assert_eq!(d.severity, Severity::Error);

        let warning = json!({
            "range": { "start": { "line": 1, "character": 8 }, "end": { "line": 1, "character": 9 } },
            "severity": 2,
            "message": "declaration uses `sorry`",
        });

        assert_eq!(diagnostic(&text, &warning).unwrap().severity, Severity::Warning);
    }

    #[test]
    fn uris_survive_spaces_and_unicode() {
        let uri = uri_of(Path::new("/Users/me/Toán học/Basic.lean"));

        assert_eq!(uri, "file:///Users/me/To%C3%A1n%20h%E1%BB%8Dc/Basic.lean");

        assert!(same_uri(&uri, "file:///Users/me/Toán%20học/Basic.lean"));

        assert!(!same_uri(&uri, "file:///Users/me/Other.lean"));
    }

    /// Pump until `done` holds, or fail after `secs`.
    fn wait_for(session: &mut Session, secs: u64, what: &str, done: impl Fn(&Session) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(secs);

        while Instant::now() < deadline {
            session.pump().expect("the server keeps running");

            if done(session) {
                return;
            }

            std::thread::sleep(Duration::from_millis(50));
        }

        panic!("timed out waiting for {what}; have {:?}", session.diagnostics());
    }

    fn slice(text: &Rope, d: &Diagnostic) -> String {
        text.slice(d.start..d.end).to_string()
    }

    /// Against a real server. Run with Lean installed:
    /// `cargo test --bin ocode -- --ignored lean_server`
    #[test]
    #[ignore = "needs a Lean toolchain"]
    fn lean_server_reports_errors_where_they_are_and_follows_edits() {
        let dir = std::env::temp_dir().join("ocode_lean_server_single");

        let _ = std::fs::remove_dir_all(&dir);

        std::fs::create_dir_all(&dir).unwrap();

        let path = dir.join("Test.lean");

        let source = "def 𝔸z : Nat := true\nexample : 1 = 2 := by rfl\ntheorem u : True := by sorry\n";

        std::fs::write(&path, source).unwrap();

        let mut text = Rope::from_str(source);

        let mut session = Session::start(&path).expect("Lean is installed");

        session.open(&path, &text);

        wait_for(&mut session, 120, "all three diagnostics", |s| s.diagnostics().len() == 3 && !s.checking());

        let found: Vec<(String, Severity)> =
            session.diagnostics().iter().map(|d| (slice(&text, d), d.severity)).collect();

        // `true` sits after `𝔸`, two UTF-16 units but one char: a wrong
        // conversion would underline `true` one character off.
        assert!(found.contains(&("true".to_string(), Severity::Error)), "{found:?}");

        assert!(found.contains(&("rfl".to_string(), Severity::Error)), "{found:?}");

        assert!(found.contains(&("u".to_string(), Severity::Warning)), "{found:?}");

        // A line inserted above moves every report down with it.
        text.insert(0, "-- a note\n");

        session.sync(&text);

        wait_for(&mut session, 60, "the reports to move", |s| {
            s.diagnostics().iter().any(|d| slice(&text, d) == "rfl" && text.char_to_line(d.start) == 2)
        });

        // Fixing the proof clears its error and leaves the others.
        let at = text.to_string().find("rfl").map(|b| text.byte_to_char(b)).unwrap();

        text.remove(at..at + 3);

        text.insert(at, "decide");

        let text = Rope::from_str(&text.to_string().replace("1 = 2", "2 = 2"));

        session.sync(&text);

        wait_for(&mut session, 60, "the fixed error to clear", |s| {
            s.diagnostics().len() == 2 && !s.checking() && s.diagnostics().iter().all(|d| slice(&text, d) != "decide")
        });

        session.stop();

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "needs a Lean toolchain"]
    fn lean_server_runs_lake_serve_in_a_project() {
        let dir = std::env::temp_dir().join("ocode_lean_server_lake");

        let _ = std::fs::remove_dir_all(&dir);

        std::fs::create_dir_all(&dir).unwrap();

        let made = std::process::Command::new(find_program("lake").expect("lake is installed"))
            .args(["new", "demo"])
            .current_dir(&dir)
            .output()
            .unwrap();

        assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));

        let path = dir.join("demo/Demo/Basic.lean");

        let source = "def hello := \"world\"\ntheorem bad : 1 = 2 := rfl\n";

        std::fs::write(&path, source).unwrap();

        let (root, lake) = project_of(&path);

        assert!(lake && root.ends_with("demo"));

        let text = Rope::from_str(source);

        let mut session = Session::start(&path).expect("Lean is installed");

        session.open(&path, &text);

        wait_for(&mut session, 180, "the error under lake serve", |s| {
            s.diagnostics().iter().any(|d| d.severity == Severity::Error && text.char_to_line(d.start) == 1)
        });

        session.stop();

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Opened through a symlink, the file's URI still lies under the root's.
    #[cfg(unix)]
    #[test]
    fn a_file_reached_through_a_symlink_stays_inside_its_project() {
        let base = std::env::temp_dir().join("ocode_lean_symlink_test");

        let _ = std::fs::remove_dir_all(&base);

        std::fs::create_dir_all(base.join("real/Learn")).unwrap();

        std::fs::write(base.join("real/lakefile.toml"), "name = \"learn\"\n").unwrap();

        std::fs::write(base.join("real/Learn/Basic.lean"), "").unwrap();

        std::os::unix::fs::symlink(base.join("real"), base.join("link")).unwrap();

        let through_link = base.join("link/Learn/Basic.lean");

        let (root, _) = project_of(&through_link);

        let doc = uri_of(&canonical(&through_link));

        assert!(doc.starts_with(&format!("{}/", uri_of(&root))), "{doc} is not under {}", uri_of(&root));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_lake_project_is_found_above_the_file() {
        let root = std::env::temp_dir().join("ocode_lean_project_test");

        let _ = std::fs::remove_dir_all(&root);

        std::fs::create_dir_all(root.join("Learn/Algebra")).unwrap();

        std::fs::write(root.join("lakefile.toml"), "name = \"learn\"\n").unwrap();

        let file = root.join("Learn/Algebra/Groups.lean");

        std::fs::write(&file, "").unwrap();

        let (found, lake) = project_of(&file);

        assert_eq!(found, std::fs::canonicalize(&root).unwrap());

        assert!(lake);

        let _ = std::fs::remove_dir_all(&root);
    }
}
