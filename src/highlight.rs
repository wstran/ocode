use std::io::Cursor;
use std::path::Path;

use ratatui::style::{Color, Modifier, Style};
use ropey::Rope;
use syntect::highlighting::{
    Color as SynColor, FontStyle, HighlightIterator, HighlightState, Highlighter, Style as SynStyle,
    Theme, ThemeSet,
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
    include_str!("../assets/syntaxes/Proto.sublime-syntax"),
    include_str!("../assets/syntaxes/GraphQL.sublime-syntax"),
    include_str!("../assets/syntaxes/HCL.sublime-syntax"),
    include_str!("../assets/syntaxes/Nix.sublime-syntax"),
    include_str!("../assets/syntaxes/Swift.sublime-syntax"),
    include_str!("../assets/syntaxes/Kotlin.sublime-syntax"),
    include_str!("../assets/syntaxes/Dart.sublime-syntax"),
    include_str!("../assets/syntaxes/Julia.sublime-syntax"),
    include_str!("../assets/syntaxes/Verilog.sublime-syntax"),
    include_str!("../assets/syntaxes/VHDL.sublime-syntax"),
    include_str!("../assets/syntaxes/Assembly.sublime-syntax"),
    include_str!("../assets/syntaxes/WebAssembly.sublime-syntax"),
    include_str!("../assets/syntaxes/Zig.sublime-syntax"),
    include_str!("../assets/syntaxes/Elixir.sublime-syntax"),
    include_str!("../assets/syntaxes/FSharp.sublime-syntax"),
    include_str!("../assets/syntaxes/PowerShell.sublime-syntax"),
    include_str!("../assets/syntaxes/Bicep.sublime-syntax"),
    include_str!("../assets/syntaxes/Tact.sublime-syntax"),
    include_str!("../assets/syntaxes/FunC.sublime-syntax"),
    include_str!("../assets/syntaxes/Clarity.sublime-syntax"),
    include_str!("../assets/syntaxes/Aiken.sublime-syntax"),
    include_str!("../assets/syntaxes/LIGO.sublime-syntax"),
    include_str!("../assets/syntaxes/Pact.sublime-syntax"),
    include_str!("../assets/syntaxes/Teal.sublime-syntax"),
    include_str!("../assets/syntaxes/Scilla.sublime-syntax"),
];

/// Variant extensions of a language syntect knows under a different one — mapped
/// to that base so e.g. `.mjs/.cjs/.jsx` light up as JavaScript and the
/// JSON-with-comments family falls back to JSON.
fn alias_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "mjs" | "cjs" | "jsx" => Some("js"),

        "jsonc" | "json5" => Some("json"),

        // Web frameworks reuse the HTML grammar — they're HTML supersets.
        "vue" | "svelte" | "astro" => Some("html"),

        // CSS preprocessors highlight as CSS for the structural parts.
        "scss" | "sass" | "less" => Some("css"),

        // MDX is Markdown with embedded JSX — Markdown covers the bulk.
        "mdx" => Some("md"),

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

    /// Chrome colors derived from the active theme (see [`UiPalette`]).
    pub fn ui_palette(&self) -> UiPalette {
        self.ui_palette_for(self.current)
    }

    /// Chrome colors derived from theme `idx`, so the picker can preview a
    /// theme's selection tint before it is applied.
    pub fn ui_palette_for(&self, idx: usize) -> UiPalette {
        let settings = &self.themes[idx.min(self.themes.len() - 1)].1.settings;

        let raw_fg = settings
            .foreground
            .unwrap_or(SynColor { r: 171, g: 178, b: 191, a: 255 });

        let bg = settings
            .background
            .unwrap_or(SynColor { r: 40, g: 44, b: 52, a: 255 });

        // Some tmThemes set a muted global foreground and rely on per-scope
        // colors for bright text; lift it to a readable floor on dark themes so
        // chrome text never sinks into the background. Light themes keep their
        // intentionally dark foreground.
        let fg = ensure_readable(raw_fg, bg);

        let selection = match settings.selection {
            Some(sel) => flatten(sel, bg),
            None => SynColor { r: 54, g: 78, b: 120, a: 255 },
        };

        UiPalette {
            fg: syn_to_color(fg),
            dim: syn_to_color(blend(fg, bg, 0.5)),
            selection: syn_to_color(selection),
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
        // Variant extensions resolve to a base language — checked first so
        // `.sass` opens as CSS (syntect's defaults wrongly claim it for Ruby
        // Haml), and the JS/JSON/HTML family aliases keep their intent.
        let by_alias = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(alias_extension)
            .and_then(|base| self.syntax_set.find_syntax_by_extension(base));

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

        by_alias
            .or(by_ext)
            .or(by_name)
            .or(by_dotfile)
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
    fn ui_palette_is_solid_and_readable() {
        let h = SyntaxHighlighter::new();

        for i in 0..h.theme_count() {
            let p = h.ui_palette_for(i);

            for c in [p.fg, p.dim, p.selection] {
                assert!(matches!(c, Color::Rgb(..)), "chrome color must be solid rgb, got {c:?}");
            }

            assert_ne!(p.fg, p.dim, "dim must read as distinct from fg (theme {i})");

            // On dark themes the primary chrome foreground must clear the
            // readability floor so status/gutter text is never near-invisible.
            if h.theme_is_dark(i) {
                if let Color::Rgb(r, g, b) = p.fg {
                    let lum = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32) / 255.0;

                    assert!(lum >= 0.59, "dark-theme fg too dim: {:?} (lum {lum:.2})", p.fg);
                }
            }
        }
    }

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
            ("api.proto", "Protocol Buffers"),
            ("schema.graphql", "GraphQL"),
            ("query.gql", "GraphQL"),
            ("main.tf", "HCL"),
            ("vars.tfvars", "HCL"),
            ("config.hcl", "HCL"),
            ("flake.nix", "Nix"),
            // JSONC / JSON5 alias onto the built-in JSON grammar.
            ("settings.jsonc", "JSON"),
            ("tsconfig.json5", "JSON"),
            ("App.swift", "Swift"),
            ("main.kt", "Kotlin"),
            ("build.gradle.kts", "Kotlin"),
            ("main.dart", "Dart"),
            ("script.jl", "Julia"),
            ("top.v", "Verilog"),
            ("top.sv", "Verilog"),
            ("core.vhd", "VHDL"),
            ("boot.asm", "Assembly"),
            ("entry.S", "Assembly"),
            ("module.wat", "WebAssembly"),
            ("main.zig", "Zig"),
            ("mod.ex", "Elixir"),
            ("mod.exs", "Elixir"),
            ("Lib.fs", "F#"),
            ("script.ps1", "PowerShell"),
            ("storage.bicep", "Bicep"),
            ("jetton.tact", "Tact"),
            ("wallet.fc", "FunC"),
            ("counter.clar", "Clarity"),
            ("validator.ak", "Aiken"),
            ("contract.ligo", "LIGO"),
            ("module.pact", "Pact"),
            ("approval.teal", "TEAL"),
            ("token.scilla", "Scilla"),
            // Aliases for web superset / preprocessor / Markdown variants.
            ("App.vue", "HTML"),
            ("Counter.svelte", "HTML"),
            ("index.astro", "HTML"),
            ("style.scss", "CSS"),
            ("style.sass", "CSS"),
            ("style.less", "CSS"),
            ("post.mdx", "Markdown"),
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
            ("syntax = \"proto3\";\nmessage M { string name = 1; }\n", "proto"),
            ("type Query { hello: String } # graphql\n", "graphql"),
            ("resource \"r\" \"x\" { count = 1 # tf\n}\n", "tf"),
            ("let pkgs = import <nixpkgs> {}; in pkgs.hello\n", "nix"),
            ("import Foundation\nlet x: Int = 1 // c\nfunc f() -> String { return \"hi\" }\n", "swift"),
            ("fun main(): Unit { val x: Int = 1; println(\"hi\") } // k\n", "kt"),
            ("void main() { final x = 1; print('hi'); } // d\n", "dart"),
            ("function f(x::Int)::Int\n  x + 1\nend # j\n", "jl"),
            ("module m(input clk, output reg q); always @(posedge clk) q <= 1; endmodule\n", "v"),
            ("entity e is port (a : in bit); end e; -- v\n", "vhd"),
            ("section .text\nglobal _start\n_start: mov eax, 1 ; asm\n", "asm"),
            ("(module (func (export \"f\") (result i32) i32.const 42)) ;; w\n", "wat"),
            ("const std = @import(\"std\");\npub fn main() void { _ = std; } // z\n", "zig"),
            ("defmodule M do\n  def hello, do: :world\nend # e\n", "ex"),
            ("let add x y = x + y // f\n", "fs"),
            ("function Get-Hello { param([string]$name); return \"hi $name\" }\n", "ps1"),
            ("param name string\nresource s 'Microsoft.Storage/storageAccounts@2021-09-01' = { name: name }\n", "bicep"),
            ("contract C { get fun greet(): String { return \"hi\"; } } // t\n", "tact"),
            (";; func\n() main() impure { return 0; }\n", "fc"),
            (";; clarity\n(define-public (greet) (ok \"hi\"))\n", "clar"),
            ("// aiken\npub fn add(x: Int, y: Int) -> Int { x + y }\n", "ak"),
            ("// ligo\nlet add = (x: int, y: int): int => x + y;\n", "ligo"),
            (";; pact\n(defun greet () \"hello\")\n", "pact"),
            ("#pragma version 6\nint 1\nreturn // teal\n", "teal"),
            ("(* scilla *)\ncontract C() transition Hello() accept end\n", "scilla"),
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

/// Colors for the app's own chrome (status text, gutter, selection), taken from
/// the active theme so the UI tracks the code. Only foregrounds and a solid
/// selection tint are derived; the terminal background is never painted.
#[derive(Clone, Copy)]
pub struct UiPalette {
    pub fg: Color,

    pub dim: Color,

    pub selection: Color,
}

/// Perceived luminance in `0.0..=1.0` (Rec. 601 weights).
fn luminance(c: SynColor) -> f32 {
    (0.299 * c.r as f32 + 0.587 * c.g as f32 + 0.114 * c.b as f32) / 255.0
}

/// On a dark theme, lift `fg` toward white until it clears a readability floor;
/// on a light theme, leave it (its dark foreground is correct on a light term).
fn ensure_readable(fg: SynColor, bg: SynColor) -> SynColor {
    const FLOOR: f32 = 0.6;

    let is_dark = u32::from(bg.r) + u32::from(bg.g) + u32::from(bg.b) < 384;

    let lum = luminance(fg);

    if !is_dark || lum >= FLOOR {
        return fg;
    }

    let white = SynColor { r: 255, g: 255, b: 255, a: 255 };

    blend(fg, white, (FLOOR - lum) / (1.0 - lum))
}

/// Mix `a` toward `b` by `t` (0.0 keeps `a`, 1.0 becomes `b`).
fn blend(a: SynColor, b: SynColor, t: f32) -> SynColor {
    let mix = |x: u8, y: u8| (x as f32 * (1.0 - t) + y as f32 * t).round() as u8;

    SynColor {
        r: mix(a.r, b.r),
        g: mix(a.g, b.g),
        b: mix(a.b, b.b),
        a: 255,
    }
}

/// Composite a translucent overlay over an opaque base into a solid color;
/// tmTheme selection colors often carry alpha meant to blend over the theme
/// background.
fn flatten(over: SynColor, base: SynColor) -> SynColor {
    let alpha = over.a as f32 / 255.0;

    let mix = |o: u8, b: u8| (o as f32 * alpha + b as f32 * (1.0 - alpha)).round() as u8;

    SynColor {
        r: mix(over.r, base.r),
        g: mix(over.g, base.g),
        b: mix(over.b, base.b),
        a: 255,
    }
}

fn syn_to_color(c: SynColor) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}
