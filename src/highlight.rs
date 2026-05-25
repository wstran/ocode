use std::io::Cursor;
use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ropey::Rope;
use syntect::highlighting::{
    FontStyle, HighlightIterator, HighlightState, Highlighter, Style as SynStyle, Theme, ThemeSet,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxDefinition, SyntaxSet};
use syntect::util::LinesWithEndings;

const DEFAULT_THEME: &str = "base16-ocean.dark";

/// Popular community color schemes bundled into the binary (sources and
/// licenses listed in assets/themes/NOTICE). Only their foreground colors are
/// used; the terminal's own background is always preserved.
const BUNDLED: &[(&str, &str)] = &[
    ("Dracula", include_str!("../assets/themes/Dracula.tmTheme")),
    ("One Dark", include_str!("../assets/themes/OneDark.tmTheme")),
    ("One Half Dark", include_str!("../assets/themes/OneHalfDark.tmTheme")),
    ("One Half Light", include_str!("../assets/themes/OneHalfLight.tmTheme")),
    ("Coldark Dark", include_str!("../assets/themes/Coldark-Dark.tmTheme")),
    ("Coldark Cold", include_str!("../assets/themes/Coldark-Cold.tmTheme")),
    ("Sublime Snazzy", include_str!("../assets/themes/Sublime-Snazzy.tmTheme")),
    ("Two Dark", include_str!("../assets/themes/TwoDark.tmTheme")),
];

/// Extra language grammars bundled on top of syntect's 75 defaults. TOML is the
/// upstream MIT grammar; the rest are minimal grammars authored for opencode
/// (the default Sublime set predates these modern languages).
const BUNDLED_SYNTAXES: &[&str] = &[
    include_str!("../assets/syntaxes/TOML.sublime-syntax"),
    include_str!("../assets/syntaxes/DotENV.sublime-syntax"),
    include_str!("../assets/syntaxes/INI.sublime-syntax"),
    include_str!("../assets/syntaxes/Dockerfile.sublime-syntax"),
    include_str!("../assets/syntaxes/TypeScript.sublime-syntax"),
    include_str!("../assets/syntaxes/Solidity.sublime-syntax"),
    include_str!("../assets/syntaxes/Move.sublime-syntax"),
    include_str!("../assets/syntaxes/Noir.sublime-syntax"),
    include_str!("../assets/syntaxes/Circom.sublime-syntax"),
    include_str!("../assets/syntaxes/Cairo.sublime-syntax"),
    include_str!("../assets/syntaxes/Vyper.sublime-syntax"),
    include_str!("../assets/syntaxes/Sway.sublime-syntax"),
    include_str!("../assets/syntaxes/Cadence.sublime-syntax"),
    include_str!("../assets/syntaxes/Leo.sublime-syntax"),
    include_str!("../assets/syntaxes/Yul.sublime-syntax"),
    include_str!("../assets/syntaxes/Huff.sublime-syntax"),
    include_str!("../assets/syntaxes/ZoKrates.sublime-syntax"),
    include_str!("../assets/syntaxes/Lean.sublime-syntax"),
];

/// Variant extensions of a language syntect knows under a different one — mapped
/// to that base so `.mjs`/`.cjs`/`.jsx` light up as JavaScript.
fn alias_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "mjs" | "cjs" | "jsx" => Some("js"),

        _ => None,
    }
}

/// Owns the immutable syntect resources (syntax definitions + the bundled
/// themes) shared by every buffer. The active theme is selectable at startup.
/// Highlighting itself is driven through a per-buffer [`HlCache`].
pub struct SyntaxHighlighter {
    syntax_set: SyntaxSet,

    themes: Vec<(String, Theme)>,

    current: usize,
}

impl SyntaxHighlighter {
    pub fn new() -> Self {
        let mut builder = SyntaxSet::load_defaults_newlines().into_builder();

        for src in BUNDLED_SYNTAXES {
            if let Ok(def) = SyntaxDefinition::load_from_str(src, true, None) {
                builder.add(def);
            }
        }

        if let Some(dir) = crate::config::syntaxes_dir() {
            let _ = builder.add_from_folder(&dir, true);
        }

        let syntax_set = builder.build();

        let mut themes: Vec<(String, Theme)> = ThemeSet::load_defaults().themes.into_iter().collect();

        for (name, data) in BUNDLED {
            if let Ok(theme) = ThemeSet::load_from_reader(&mut Cursor::new(data.as_bytes())) {
                themes.push((name.to_string(), theme));
            }
        }

        if let Some(dir) = crate::config::themes_dir() {
            load_user_themes(&dir, &mut themes);
        }

        themes.sort_by_key(|t| t.0.to_lowercase());

        let current = themes
            .iter()
            .position(|(name, _)| name == DEFAULT_THEME)
            .unwrap_or(0);

        Self {
            syntax_set,
            themes,
            current,
        }
    }

    pub fn theme_index(&self, name: &str) -> Option<usize> {
        self.themes.iter().position(|(n, _)| n == name)
    }

    /// Whether a theme was designed for a dark background, inferred from its
    /// declared background luminance. Used only as a label in the picker — the
    /// background itself is never applied.
    pub fn theme_is_dark(&self, idx: usize) -> bool {
        self.themes
            .get(idx)
            .and_then(|(_, t)| t.settings.background)
            .map(|c| u32::from(c.r) + u32::from(c.g) + u32::from(c.b) < 384)
            .unwrap_or(true)
    }

    pub fn theme_names(&self) -> Vec<&str> {
        self.themes.iter().map(|(name, _)| name.as_str()).collect()
    }

    pub fn theme_count(&self) -> usize {
        self.themes.len()
    }

    pub fn current_theme(&self) -> usize {
        self.current
    }

    pub fn set_theme(&mut self, idx: usize) {
        if idx < self.themes.len() {
            self.current = idx;
        }
    }

    /// One-shot highlight of a small snippet with an arbitrary theme, used to
    /// preview styles in the picker. Not cached — only for short text.
    pub fn highlight_block(&self, code: &str, ext: &str, theme_idx: usize) -> Vec<Vec<(Style, String)>> {
        let syntax = self
            .syntax_set
            .find_syntax_by_extension(ext)
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let theme = &self.themes[theme_idx.min(self.themes.len() - 1)].1;

        let highlighter = Highlighter::new(theme);

        let mut parse = ParseState::new(syntax);

        let mut hl = HighlightState::new(&highlighter, ScopeStack::new());

        let mut out = Vec::new();

        for line in LinesWithEndings::from(code) {
            let spans = match parse.parse_line(line, &self.syntax_set) {
                Ok(ops) => HighlightIterator::new(&mut hl, &ops, line, &highlighter)
                    .filter_map(|(syn, text)| convert_span(syn, text))
                    .collect(),

                Err(_) => Vec::new(),
            };

            out.push(spans);
        }

        out
    }

    /// Resolve the syntax name for a path via its extension / file name so the
    /// cache can later look the syntax back up by name.
    pub fn syntax_name_for_path(&self, path: &Path) -> String {
        let by_ext = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|e| self.syntax_set.find_syntax_by_extension(e));

        let name = path.file_name().and_then(|n| n.to_str());

        let by_name = name.and_then(|n| self.syntax_set.find_syntax_by_extension(n));

        // Dotfiles like `.env` have no extension; match the name without its dot.
        let by_dotfile = name
            .and_then(|n| n.strip_prefix('.'))
            .and_then(|n| self.syntax_set.find_syntax_by_extension(n));

        // Variant extensions (.mjs/.cjs/.jsx) reuse the base language's syntax.
        let by_alias = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(alias_extension)
            .and_then(|base| self.syntax_set.find_syntax_by_extension(base));

        by_ext
            .or(by_name)
            .or(by_dotfile)
            .or(by_alias)
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text())
            .name
            .clone()
    }

    /// Highlight lazily up to and including `target_line`, resuming from the
    /// last cached parser checkpoint. Only the newly requested region is parsed.
    pub fn ensure(&self, cache: &mut HlCache, rope: &Rope, target_line: usize) {
        let syntax = self
            .syntax_set
            .find_syntax_by_name(&cache.syntax_name)
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let highlighter = Highlighter::new(&self.themes[self.current].1);

        if cache.states.is_empty() {
            let initial = (
                ParseState::new(syntax),
                HighlightState::new(&highlighter, ScopeStack::new()),
            );

            cache.states.push(initial);
        }

        let total = rope.len_lines();

        while cache.lines.len() <= target_line && cache.lines.len() < total {
            let i = cache.lines.len();

            let (mut parse, mut hl) = cache.states[i].clone();

            let raw = rope.line(i).to_string();

            let spans = match parse.parse_line(&raw, &self.syntax_set) {
                Ok(ops) => HighlightIterator::new(&mut hl, &ops, &raw, &highlighter)
                    .filter_map(|(syn, text)| convert_span(syn, text))
                    .collect(),

                Err(_) => {
                    let trimmed = raw.trim_end_matches(['\n', '\r']);

                    if trimmed.is_empty() {
                        Vec::new()
                    } else {
                        vec![(Style::default(), trimmed.to_string())]
                    }
                }
            };

            cache.lines.push(spans);

            cache.states.push((parse, hl));
        }
    }
}

/// Per-buffer highlight cache. `states[i]` is the parser/highlighter state
/// *before* line `i`; `lines[i]` holds the styled spans of line `i`.
pub struct HlCache {
    syntax_name: String,

    states: Vec<(ParseState, HighlightState)>,

    lines: Vec<Vec<(Style, String)>>,
}

impl HlCache {
    pub fn new(syntax_name: String) -> Self {
        Self {
            syntax_name,
            states: Vec::new(),
            lines: Vec::new(),
        }
    }

    /// Spans for a line, or `None` if it has not been highlighted yet (the
    /// caller is expected to call [`SyntaxHighlighter::ensure`] first).
    pub fn line(&self, idx: usize) -> Option<&[(Style, String)]> {
        self.lines.get(idx).map(|v| v.as_slice())
    }

    /// Drop every cached line from `from` onward so it gets recomputed. The
    /// parser checkpoint at `from` is preserved as the resume point.
    pub fn invalidate(&mut self, from: usize) {
        self.lines.truncate(from);

        self.states.truncate(from + 1);
    }
}

fn convert_span(syn: SynStyle, text: &str) -> Option<(Style, String)> {
    let trimmed = text.trim_end_matches(['\n', '\r']);

    if trimmed.is_empty() {
        return None;
    }

    let fg = Color::Rgb(
        syn.foreground.r,
        syn.foreground.g,
        syn.foreground.b,
    );

    let mut style = Style::default().fg(fg);

    if syn.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }

    if syn.font_style.contains(FontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }

    if syn.font_style.contains(FontStyle::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }

    Some((style, trimmed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_bundled_languages() {
        let h = SyntaxHighlighter::new();

        assert_eq!(h.syntax_name_for_path(Path::new("Cargo.toml")), "TOML");

        assert_eq!(h.syntax_name_for_path(Path::new(".env")), "DotENV");

        assert_eq!(h.syntax_name_for_path(Path::new("prod.env")), "DotENV");

        assert_eq!(h.syntax_name_for_path(Path::new("app.ini")), "INI");

        assert_eq!(h.syntax_name_for_path(Path::new("Dockerfile")), "Dockerfile");

        assert_eq!(h.syntax_name_for_path(Path::new("data.json")), "JSON");

        assert_eq!(h.syntax_name_for_path(Path::new("conf.yaml")), "YAML");
    }

    #[test]
    fn resolves_modern_languages_and_variants() {
        let h = SyntaxHighlighter::new();

        for (file, want) in [
            ("app.ts", "TypeScript"),
            ("App.tsx", "TypeScript"),
            ("mod.mts", "TypeScript"),
            ("mod.cts", "TypeScript"),
            ("Token.sol", "Solidity"),
            ("coin.move", "Move"),
            ("main.nr", "Noir"),
            ("circuit.circom", "Circom"),
            ("lib.cairo", "Cairo"),
            ("token.vy", "Vyper"),
            ("main.sw", "Sway"),
            ("nft.cdc", "Cadence"),
            ("main.leo", "Leo"),
            ("opt.yul", "Yul"),
            ("Main.huff", "Huff"),
            ("circuit.zok", "ZoKrates"),
            ("Proof.lean", "Lean"),
            // JS variants alias onto the built-in JavaScript grammar.
            ("server.mjs", "JavaScript"),
            ("config.cjs", "JavaScript"),
            ("App.jsx", "JavaScript"),
        ] {
            assert_eq!(h.syntax_name_for_path(Path::new(file)), want, "for {file}");
        }
    }

    #[test]
    fn modern_grammars_highlight_without_error() {
        let h = SyntaxHighlighter::new();

        // Each minimal grammar must load and produce >1 styled span (i.e. it
        // actually tokenised, not fell back to one plain-text run).
        for (code, ext) in [
            ("const x: number = 1; // hi\n", "ts"),
            ("contract C { uint256 public x; }\n", "sol"),
            ("module m::a { fun f() { let x = 1; } }\n", "move"),
            ("fn main() { let x: Field = 1; }\n", "nr"),
            ("template T() { signal input a; a <== 1; }\n", "circom"),
            ("fn main() -> felt252 { let x = 1; return x; }\n", "cairo"),
            ("@external\ndef foo() -> uint256: return 1\n", "vy"),
            ("contract C { fn main() -> u64 { return 1; } }\n", "sw"),
            ("pub contract C { pub fun main(): Int { return 1 } }\n", "cdc"),
            ("program p.aleo { transition main() -> u32 { return 1u32; } }\n", "leo"),
            ("object \"C\" { code { let x := mload(0x40) } }\n", "yul"),
            ("#define macro MAIN() = takes(0) returns(0) { 0x00 mload }\n", "huff"),
            ("def main(field a) -> field { return a; }\n", "zok"),
            ("theorem t : 1 = 1 := by rfl -- proof\n", "lean"),
        ] {
            let spans = h.highlight_block(code, ext, h.current);

            let count: usize = spans.iter().map(|l| l.len()).sum();

            assert!(count > 1, "grammar for .{ext} did not tokenise ({count} spans)");
        }
    }
}

fn load_user_themes(dir: &Path, themes: &mut Vec<(String, Theme)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("tmTheme") {
            continue;
        }

        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };

        if let Ok(theme) = ThemeSet::load_from_reader(&mut Cursor::new(bytes)) {
            let name = theme.name.clone().unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("custom")
                    .to_string()
            });

            themes.push((name, theme));
        }
    }
}
