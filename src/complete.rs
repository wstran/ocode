//! The list of names Lean offers while a name is typed in a Lean file. It opens
//! by itself after a `.` or two letters of a name, or on Ctrl+Space, narrows as
//! the name grows, and Tab or Enter takes the selected one.

use crate::lean::Completion;

/// Letters of a name typed before the list is asked for and opens by itself.
/// One would ask for nearly every word, and single-letter names such as `h`
/// and `n` are most of the words in a proof.
pub const AUTO_AFTER: usize = 2;

/// Rows of names shown at once.
pub const ROWS: usize = 8;

/// Names kept after narrowing; nobody scrolls through more.
const KEEP: usize = 200;

pub struct Popup {
    /// Char index where the word being completed starts.
    pub start: usize,

    /// Indexes into the offered names that fit what is typed, best first.
    pub matches: Vec<usize>,

    pub selected: usize,

    /// First row shown, so the selection stays in view.
    pub scroll: usize,

    /// Opened on purpose, by Ctrl+Space or a `.`, so shown before two letters.
    pub explicit: bool,

    /// Whether it is on screen, worked out once a turn of the loop.
    pub shown: bool,

    /// What the matches were worked out for: the word and how many names
    /// were offered, so they are not ranked again while neither changes.
    narrowed_for: Option<(String, usize)>,

    /// Every offered name that fits the word, not only the best `KEEP`: the
    /// names that fit a longer word are among them, so typing on searches
    /// these instead of all of Mathlib's seventy-odd thousand again.
    fits: Vec<usize>,
}

impl Popup {
    pub fn new(start: usize, explicit: bool) -> Self {
        Self {
            start,
            matches: Vec::new(),
            selected: 0,
            scroll: 0,
            explicit,
            shown: false,
            narrowed_for: None,
            fits: Vec::new(),
        }
    }

    /// Whether there is anything to show for a word of `typed` chars.
    pub fn visible(&self, typed: usize) -> bool {
        !self.matches.is_empty() && (self.explicit || typed >= AUTO_AFTER)
    }

    /// Move the selection by `delta`, wrapping at either end.
    pub fn step(&mut self, delta: isize) {
        let len = self.matches.len() as isize;

        if len == 0 {
            return;
        }

        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;

        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + ROWS {
            self.scroll = self.selected + 1 - ROWS;
        }
    }

    /// Narrow to the names that fit `typed`, keeping the selection on the same
    /// name when it still fits.
    pub fn narrow(&mut self, items: &[Completion], typed: &str) {
        let key = (typed.to_string(), items.len());

        if self.narrowed_for.as_ref() == Some(&key) {
            return;
        }

        let pool = match self.narrowed_for.take() {
            Some((before, len)) if len == items.len() && typed.starts_with(&before) => std::mem::take(&mut self.fits),

            _ => (0..items.len()).collect(),
        };

        self.narrowed_for = Some(key);

        let kept = self.matches.get(self.selected).copied();

        let (best, fits) = rank_among(items, typed, pool);

        self.matches = best;

        self.fits = fits;

        self.selected = kept.and_then(|k| self.matches.iter().position(|&m| m == k)).unwrap_or(0);

        self.scroll = self.scroll.min(self.selected);

        if self.selected >= self.scroll + ROWS {
            self.scroll = self.selected + 1 - ROWS;
        }
    }
}

/// Where the name the caret is in starts, scanning back over name chars.
pub fn word_start(text: &ropey::Rope, at: usize) -> usize {
    let mut start = at;

    while start > 0 && is_name_char(text.char(start - 1)) {
        start -= 1;
    }

    start
}

/// A char that can be part of a Lean name. The dot is left out: it separates
/// the parts of a name, and a completion replaces only the last part.
pub fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '\'' | '!' | '?')
}

/// The offered names that fit `typed`, best first. Those that start with it
/// come first, then those with a dotted part that does (`succ_p` finds
/// `Nat.succ_pos`), then the same two ignoring case, then those that contain
/// its letters in order. Within a tier, shorter names come first.
#[cfg(test)]
pub fn rank(items: &[Completion], typed: &str) -> Vec<usize> {
    rank_among(items, typed, (0..items.len()).collect()).0
}

/// Rank the names at `pool` against `typed`: the best `KEEP` in order, and
/// every one that fits at all, to narrow from as the word grows.
fn rank_among(items: &[Completion], typed: &str, pool: Vec<usize>) -> (Vec<usize>, Vec<usize>) {
    let folded: String = typed.chars().map(fold).collect();

    let mut scored: Vec<(u8, usize, usize)> = pool
        .into_iter()
        .filter_map(|i| {
            let label = items[i].label.as_str();

            Some((tier(label, typed, &folded)?, label.chars().count(), i))
        })
        .collect();

    let fits = scored.iter().map(|&(_, _, i)| i).collect();

    let order = |a: &(u8, usize, usize), b: &(u8, usize, usize)| {
        (a.0, a.1, items[a.2].label.as_str()).cmp(&(b.0, b.1, items[b.2].label.as_str()))
    };

    // Only the best few are shown; sorting the rest of thousands is waste.
    if scored.len() > KEEP {
        scored.select_nth_unstable_by(KEEP, order);

        scored.truncate(KEEP);
    }

    scored.sort_unstable_by(order);

    (scored.into_iter().map(|(_, _, i)| i).collect(), fits)
}

/// A char with case folded, without building a lowered copy of each name: this
/// runs over every offered name on every keystroke.
fn fold(c: char) -> char {
    if c.is_ascii() { c.to_ascii_lowercase() } else { c.to_lowercase().next().unwrap_or(c) }
}

fn starts_folded(text: &str, folded: &str) -> bool {
    let mut chars = text.chars().map(fold);

    folded.chars().all(|f| chars.next() == Some(f))
}

fn tier(label: &str, typed: &str, folded: &str) -> Option<u8> {
    if label.starts_with(typed) {
        return Some(0);
    }

    if label.split('.').any(|part| part.starts_with(typed)) {
        return Some(1);
    }

    if starts_folded(label, folded) {
        return Some(2);
    }

    if label.split('.').any(|part| starts_folded(part, folded)) {
        return Some(3);
    }

    let mut chars = label.chars().map(fold);

    folded.chars().all(|f| chars.any(|c| c == f)).then_some(4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(labels: &[&str]) -> Vec<Completion> {
        labels.iter().map(|l| Completion::named(l)).collect()
    }

    fn ranked(labels: &[&str], typed: &str) -> Vec<String> {
        let items = items(labels);

        rank(&items, typed).into_iter().map(|i| items[i].label.clone()).collect()
    }

    #[test]
    fn names_that_start_with_it_come_first() {
        let labels = ["Nat.succ_pos", "succ_pos_of_lt", "sub_pos", "succ", "Succ", "le_succ_pos"];

        assert_eq!(
            ranked(&labels, "succ"),
            ["succ", "succ_pos_of_lt", "Nat.succ_pos", "Succ", "le_succ_pos"],
            "exact start, then a dotted part, then ignoring case, then letters in order"
        );

        // All four only hold the letters in order, so length decides.
        assert_eq!(ranked(&labels, "spos"), ["sub_pos", "le_succ_pos", "Nat.succ_pos", "succ_pos_of_lt"]);

        assert!(ranked(&labels, "xyz").is_empty());
    }

    /// Typing on narrows from the last fits rather than from every name, and
    /// must find exactly what ranking from scratch finds.
    #[test]
    fn narrowing_as_the_word_grows_matches_ranking_from_scratch() {
        let labels: Vec<String> = (0..3000).map(|i| format!("Ns{}.add_{}_comm{}", i % 7, ["co", "cm", "x"][i % 3], i)).collect();

        let items: Vec<Completion> = labels.iter().map(|l| Completion::named(l)).collect();

        let mut popup = Popup::new(0, false);

        for typed in ["a", "ad", "add", "add_", "add_c", "add_co", "add_com", "Add_Co", "zz"] {
            popup.narrow(&items, typed);

            assert_eq!(popup.matches, rank(&items, typed), "after typing {typed:?}");
        }
    }

    #[test]
    fn an_empty_word_offers_everything_shortest_first() {
        assert_eq!(ranked(&["map", "length", "mem"], ""), ["map", "mem", "length"]);
    }

    #[test]
    fn the_selection_wraps_and_stays_in_view() {
        let mut popup = Popup::new(0, true);

        popup.matches = (0..20).collect();

        popup.step(-1);

        assert_eq!(popup.selected, 19);

        assert_eq!(popup.scroll, 19 + 1 - ROWS, "the last row is in view");

        popup.step(1);

        assert_eq!((popup.selected, popup.scroll), (0, 0));
    }

    #[test]
    fn narrowing_keeps_the_selected_name() {
        let items = items(&["map", "mapM", "max", "mem"]);

        let mut popup = Popup::new(0, false);

        popup.narrow(&items, "m");

        popup.step(1);

        let chosen = items[popup.matches[popup.selected]].label.clone();

        popup.narrow(&items, "ma");

        assert_eq!(items[popup.matches[popup.selected]].label, chosen);

        assert!(!popup.visible(1) && popup.visible(2), "two letters before it shows by itself");
    }
}
