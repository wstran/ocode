//! Lean's unicode input: typing `\forall` writes `∀`, the way Lean's own
//! editors do it. The table is the one the official VS Code extension ships
//! (assets/lean, Apache-2.0) and the rules follow its `AbbreviationRewriter`,
//! so an abbreviation learnt in one editor works in the other.

use std::sync::OnceLock;

use crate::buffer::Buffer;

const TABLE_JSON: &str = include_str!("../assets/lean/abbreviations.json");

/// Marks where a replacement leaves the cursor: `\<>` gives `⟨⟩` with the
/// cursor between the brackets.
const CURSOR: &str = "$CURSOR";

/// The abbreviations in the order upstream declares them, which matters: when
/// two of the same length both fit, the earlier one wins.
pub struct Table {
    entries: Vec<(String, String)>,
}

impl Table {
    fn parse(json: &str) -> serde_json::Result<Self> {
        struct Ordered(Vec<(String, String)>);

        struct OrderedVisitor;

        impl<'de> serde::de::Visitor<'de> for OrderedVisitor {
            type Value = Ordered;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object mapping abbreviations to symbols")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Ordered, A::Error> {
                let mut entries = Vec::new();

                while let Some(entry) = map.next_entry::<String, String>()? {
                    entries.push(entry);
                }

                Ok(Ordered(entries))
            }
        }

        impl<'de> serde::Deserialize<'de> for Ordered {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                d.deserialize_map(OrderedVisitor)
            }
        }

        serde_json::from_str::<Ordered>(json).map(|ordered| Table { entries: ordered.0 })
    }

    fn starting_with<'a>(&'a self, prefix: &str) -> impl Iterator<Item = &'a (String, String)> {
        self.entries.iter().filter(move |(abbrev, _)| abbrev.starts_with(prefix))
    }

    /// Whether some abbreviation still begins with `prefix`.
    fn continues(&self, prefix: &str) -> bool {
        self.starting_with(prefix).next().is_some()
    }

    /// Complete, and the only abbreviation it could still grow into, so there
    /// is nothing left to wait for.
    fn is_settled(&self, abbrev: &str) -> bool {
        let mut fits = self.starting_with(abbrev);

        fits.next().is_some_and(|(a, _)| a == abbrev) && fits.next().is_none()
    }

    /// What `\abbrev` turns into: the symbol of the shortest abbreviation that
    /// starts with it, or else its longest prefix that has one, with the rest
    /// kept as typed (`\alp7` gives `α7`).
    pub fn replacement(&self, abbrev: &str) -> Option<String> {
        if abbrev.is_empty() {
            return None;
        }

        // `min_by_key` keeps the first of equals, which is the declared order.
        if let Some((_, symbol)) = self.starting_with(abbrev).min_by_key(|(a, _)| a.len()) {
            return Some(symbol.clone());
        }

        let mut chars = abbrev.chars();

        let last = chars.next_back()?;

        self.replacement(chars.as_str()).map(|head| format!("{head}{last}"))
    }

    /// Every abbreviation that types `symbol`, shortest first.
    pub fn ways_to_type(&self, symbol: &str) -> Vec<&str> {
        let mut ways: Vec<&str> = self
            .entries
            .iter()
            .filter(|(_, s)| s == symbol)
            .map(|(a, _)| a.as_str())
            .collect();

        ways.sort_by_key(|a| a.len());

        ways
    }

    /// The distinct symbols a half-typed abbreviation could still become, each
    /// with its shortest abbreviation, shortest first.
    pub fn candidates(&self, prefix: &str, limit: usize) -> Vec<(&str, &str)> {
        let mut fits: Vec<&(String, String)> = self.starting_with(prefix).collect();

        fits.sort_by_key(|(a, _)| a.len());

        let mut out: Vec<(&str, &str)> = Vec::new();

        for (abbrev, symbol) in fits {
            if out.len() == limit {
                break;
            }

            if !out.iter().any(|(_, s)| s == symbol) {
                out.push((abbrev.as_str(), symbol.as_str()));
            }
        }

        out
    }
}

/// The bundled table, parsed the first time a Lean file needs it.
pub fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();

    TABLE.get_or_init(|| Table::parse(TABLE_JSON).expect("the bundled abbreviation table is valid"))
}

/// An abbreviation being typed: the `\` that opened it and what follows.
#[derive(Clone, Debug, PartialEq)]
pub struct Tracked {
    /// Char index of the `\`.
    pub start: usize,

    pub text: String,
}

impl Tracked {
    /// Char index just past the abbreviation, where the cursor sits while it
    /// is being typed.
    pub fn end(&self) -> usize {
        self.start + 1 + self.text.chars().count()
    }
}

/// Type `ch` into a Lean buffer.
pub fn type_char(buf: &mut Buffer, ch: char) {
    if let Some(tracked) = buf.abbrev.take() {
        let mut longer = tracked.text.clone();

        longer.push(ch);

        if buf.char_idx() == tracked.end() && buf.selection().is_none() && table().continues(&longer) {
            buf.insert_char(ch);

            let tracked = Tracked { start: tracked.start, text: longer };

            if table().is_settled(&tracked.text) {
                commit(buf, &tracked, true);
            } else {
                buf.abbrev = Some(tracked);
            }

            // Returning here also keeps the `\` of `\\` from opening another.
            return;
        }

        // `ch` cannot extend it, so the abbreviation is finished and `ch` is
        // typed after it: `\a` then a space gives `α `.
        commit(buf, &tracked, false);
    }

    buf.insert_char(ch);

    if ch == '\\' {
        buf.abbrev = Some(Tracked { start: buf.char_idx() - 1, text: String::new() });
    }
}

/// Backspace inside an abbreviation shortens it; deleting its `\` ends it.
pub fn backspace(buf: &mut Buffer) {
    if let Some(mut tracked) = buf.abbrev.take() {
        if buf.char_idx() == tracked.end() && buf.selection().is_none() {
            buf.backspace();

            if tracked.text.pop().is_some() {
                buf.abbrev = Some(tracked);
            }

            return;
        }

        commit(buf, &tracked, false);
    }

    buf.backspace();
}

/// Tab replaces an abbreviation before it is complete, with the shortest one
/// that fits. Returns `false` when none was being typed, so Tab indents.
pub fn complete(buf: &mut Buffer) -> bool {
    let Some(tracked) = buf.abbrev.take() else {
        return false;
    };

    commit(buf, &tracked, true);

    true
}

/// Replace a pending abbreviation before something that is not part of
/// typing it, so Enter, a save or a paste see `∀` rather than `\forall`.
pub fn commit_pending(buf: &mut Buffer) {
    if let Some(tracked) = buf.abbrev.take() {
        commit(buf, &tracked, false);
    }
}

/// Esc keeps an abbreviation as typed, for the times a backslash is meant
/// literally (`\t` in a string would otherwise become `▸`).
pub fn cancel(buf: &mut Buffer) -> bool {
    buf.abbrev.take().is_some()
}

fn commit(buf: &mut Buffer, tracked: &Tracked, cursor_to_marker: bool) {
    // An undo or a reload may have changed the text underneath; then there is
    // nothing that can safely be replaced.
    if !buf.holds_abbreviation(tracked) {
        return;
    }

    let Some(symbol) = table().replacement(&tracked.text) else {
        return;
    };

    let marker = symbol.find(CURSOR).map(|at| symbol[..at].chars().count());

    let text = symbol.replacen(CURSOR, "", 1);

    // `\n` stands for itself, so a newline escape in a string survives. There
    // is nothing to replace, and an undo step for it would undo nothing.
    if marker.is_none() && text.strip_prefix('\\') == Some(tracked.text.as_str()) {
        return;
    }

    let range = tracked.start..tracked.end();

    let cursor = buf.char_idx();

    let cursor = match marker {
        Some(offset) if cursor_to_marker && cursor == range.end => range.start + offset,

        _ if cursor >= range.end => cursor - range.len() + text.chars().count(),

        _ => cursor,
    };

    buf.replace_chars(range, &text, cursor);
}

/// What the status bar says about Lean input: while an abbreviation is being
/// typed, what it will become and what else it could; otherwise how to type
/// the symbol at the cursor.
pub fn hint(buf: &Buffer) -> Option<String> {
    if !buf.is_lean() {
        return None;
    }

    let table = table();

    if let Some(tracked) = &buf.abbrev {
        let becomes = table.replacement(&tracked.text)?.replacen(CURSOR, "", 1);

        let mut hint = format!("\\{} → {becomes}", tracked.text);

        let others: Vec<String> = table
            .candidates(&tracked.text, 5)
            .into_iter()
            .map(|(abbrev, symbol)| (abbrev, symbol.replacen(CURSOR, "", 1)))
            .filter(|(_, symbol)| *symbol != becomes)
            .take(4)
            .map(|(abbrev, symbol)| format!("{symbol} \\{abbrev}"))
            .collect();

        if !others.is_empty() {
            hint.push_str("  ·  ");

            hint.push_str(&others.join("  "));
        }

        return Some(hint);
    }

    // The character under the cursor, or failing that the one just typed.
    let idx = buf.char_idx();

    let at = |i: usize| (i < buf.rope.len_chars()).then(|| buf.rope.char(i));

    [at(idx), idx.checked_sub(1).and_then(at)]
        .into_iter()
        .flatten()
        .filter(|c| !c.is_ascii())
        .find_map(|c| {
            let ways = table.ways_to_type(&c.to_string());

            (!ways.is_empty()).then(|| {
                let ways: Vec<String> = ways.iter().take(3).map(|w| format!("\\{w}")).collect();

                format!("{c}  {}", ways.join("  "))
            })
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// An empty Lean buffer. The path does not exist, so nothing is read.
    fn lean() -> Buffer {
        Buffer::open(PathBuf::from("/nonexistent/ocode-abbrev-test.lean"), "Lean".to_string()).unwrap()
    }

    fn type_str(buf: &mut Buffer, text: &str) {
        for ch in text.chars() {
            type_char(buf, ch);
        }
    }

    fn text(buf: &Buffer) -> String {
        buf.rope.to_string()
    }

    #[test]
    fn table_keeps_upstream_order_and_size() {
        let table = table();

        assert_eq!(table.entries.len(), 1865, "the vscode-lean4 table has 1865 abbreviations");

        assert_eq!(table.symbol("forall"), Some("∀"));

        assert_eq!(table.symbol("\\"), Some("\\"), "the escaped backslash key parses");
    }

    impl Table {
        fn symbol(&self, abbrev: &str) -> Option<&str> {
            self.entries.iter().find(|(a, _)| a == abbrev).map(|(_, s)| s.as_str())
        }
    }

    /// The replacement rules of vscode-lean4's AbbreviationProvider.
    #[test]
    fn replacement_follows_upstream() {
        let table = table();

        // The shortest abbreviation that starts with what was typed.
        assert_eq!(table.replacement("al").as_deref(), Some("∀"), "\\al is \\all, the shortest");

        assert_eq!(table.replacement("alpha").as_deref(), Some("α"));

        // An unknown tail is kept after the longest prefix that has a symbol.
        assert_eq!(table.replacement("alp7").as_deref(), Some("α7"));

        assert_eq!(table.replacement(""), None);

        // `\n` is kept as typed, so a newline escape in a string survives.
        assert_eq!(table.replacement("n").as_deref(), Some("\\n"));
    }

    /// Replacing `\n` with itself would leave an undo step that undoes nothing.
    #[test]
    fn an_abbreviation_that_stands_for_itself_leaves_no_undo_step() {
        let mut b = lean();

        type_str(&mut b, "s!\"a\\n");

        commit_pending(&mut b);

        assert_eq!(text(&b), "s!\"a\\n");

        b.undo();

        assert_eq!(text(&b), "s!\"a\\", "the first undo takes back typing, not a no-op");
    }

    #[test]
    fn a_complete_unique_abbreviation_is_replaced_as_it_is_typed() {
        let mut b = lean();

        type_str(&mut b, "\\forall");

        assert_eq!(text(&b), "∀");

        assert!(b.abbrev.is_none());

        assert_eq!(b.char_idx(), 1);
    }

    #[test]
    fn a_character_that_cannot_continue_finishes_the_abbreviation() {
        let mut b = lean();

        type_str(&mut b, "x : \\a ");

        assert_eq!(text(&b), "x : α ");

        assert_eq!(b.char_idx(), 6);
    }

    #[test]
    fn abbreviations_follow_one_another() {
        let mut b = lean();

        type_str(&mut b, "\\a\\b");

        assert_eq!(text(&b), "α\\b", "the second backslash finished the first and opened its own");

        assert!(complete(&mut b));

        assert_eq!(text(&b), "αβ");
    }

    #[test]
    fn tab_takes_the_shortest_that_fits() {
        let mut b = lean();

        type_str(&mut b, "\\al");

        assert_eq!(text(&b), "\\al", "\\al could still become \\alpha, so it waits");

        assert!(complete(&mut b));

        assert_eq!(text(&b), "∀");

        assert!(!complete(&mut b), "with nothing pending Tab is free to indent");
    }

    #[test]
    fn two_backslashes_make_one() {
        let mut b = lean();

        type_str(&mut b, "\\\\");

        assert_eq!(text(&b), "\\");

        assert!(b.abbrev.is_none(), "the backslash that came out of it opens nothing");
    }

    #[test]
    fn a_cursor_marker_lands_between_the_brackets() {
        let mut b = lean();

        type_str(&mut b, "\\<>");

        assert_eq!(text(&b), "⟨⟩");

        assert_eq!(b.char_idx(), 1);

        type_str(&mut b, "x");

        assert_eq!(text(&b), "⟨x⟩");
    }

    #[test]
    fn a_backslash_with_nothing_after_it_stays() {
        let mut b = lean();

        type_str(&mut b, "a \\ b");

        assert_eq!(text(&b), "a \\ b");
    }

    #[test]
    fn backspace_edits_the_abbreviation_and_can_end_it() {
        let mut b = lean();

        type_str(&mut b, "\\alx");

        // `x` cannot follow `al`, so `\al` became `∀` and `x` came after it.
        assert_eq!(text(&b), "∀x");

        let mut b = lean();

        type_str(&mut b, "\\alp");

        backspace(&mut b);

        assert_eq!(b.abbrev.as_ref().map(|t| t.text.as_str()), Some("al"));

        backspace(&mut b);

        backspace(&mut b);

        backspace(&mut b);

        assert_eq!(text(&b), "");

        assert!(b.abbrev.is_none(), "deleting the backslash ends it");
    }

    #[test]
    fn esc_keeps_what_was_typed() {
        let mut b = lean();

        type_str(&mut b, "\"\\t");

        assert!(cancel(&mut b));

        type_str(&mut b, "\"");

        assert_eq!(text(&b), "\"\\t\"");
    }

    #[test]
    fn commit_before_another_action_and_undo_restores_the_abbreviation() {
        let mut b = lean();

        type_str(&mut b, "\\to");

        commit_pending(&mut b);

        assert_eq!(text(&b), "→");

        b.undo();

        assert_eq!(text(&b), "\\to", "undo brings back what was typed");

        assert!(b.abbrev.is_none(), "and does not resume the abbreviation");

        commit_pending(&mut b);

        assert_eq!(text(&b), "\\to");
    }

    #[test]
    fn a_stale_abbreviation_is_never_replaced() {
        let mut b = lean();

        type_str(&mut b, "\\al");

        // The text changes underneath, as a reload from disk would.
        let stale = b.abbrev.clone();

        b.select_all();

        b.insert_char('z');

        b.abbrev = stale;

        commit_pending(&mut b);

        assert_eq!(text(&b), "z");
    }

    #[test]
    fn hint_shows_what_is_coming_and_how_to_type_a_symbol() {
        let mut b = lean();

        type_str(&mut b, "\\al");

        let typing = hint(&b).unwrap();

        assert!(typing.starts_with("\\al → ∀"), "{typing}");

        assert!(typing.contains("α \\alpha"), "{typing}");

        complete(&mut b);

        assert_eq!(hint(&b).as_deref(), Some("∀  \\all  \\forall"));
    }
}
