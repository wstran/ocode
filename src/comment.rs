//! Per-language comment tokens for the toggle-comment command.
//!
//! Line tokens for the grammars bundled with opencode are verified against the
//! grammar files themselves by `tests::table_matches_bundled_grammars`, which
//! reads each `comment.line.*` scope and checks it against this table. The
//! remaining entries are standard tokens for syntect's built-in languages.
//! Anything not listed toggles nothing, so an unknown language can never have
//! the wrong token inserted into it.

#[derive(Clone, Copy, Default)]
pub struct CommentStyle {
    /// Line comment prefix, e.g. `//`.
    pub line: Option<&'static str>,

    /// Block comment (open, close), e.g. `("/*", "*/")`.
    pub block: Option<(&'static str, &'static str)>,
}

const SLASH: CommentStyle = CommentStyle { line: Some("//"), block: Some(("/*", "*/")) };
const HASH: CommentStyle = CommentStyle { line: Some("#"), block: None };
const DASH: CommentStyle = CommentStyle { line: Some("--"), block: None };
const SEMI2: CommentStyle = CommentStyle { line: Some(";;"), block: None };
const SEMI: CommentStyle = CommentStyle { line: Some(";"), block: None };
const PERCENT: CommentStyle = CommentStyle { line: Some("%"), block: None };
const PAREN: CommentStyle = CommentStyle { line: None, block: Some(("(*", "*)")) };
const XML: CommentStyle = CommentStyle { line: None, block: Some(("<!--", "-->")) };
const CSS: CommentStyle = CommentStyle { line: None, block: Some(("/*", "*/")) };

/// The comment style for a syntect syntax name, or an empty style (toggles
/// nothing) for a language with no known comment token.
pub fn for_syntax(name: &str) -> CommentStyle {
    match name {
        // Bundled grammars, C-family line comment (verified against the files).
        "TypeScript" | "Solidity" | "Move" | "Noir" | "Circom" | "Cairo" | "Sway"
        | "Cadence" | "Leo" | "Yul" | "Huff" | "ZoKrates" | "Protocol Buffers" | "Swift"
        | "Kotlin" | "Dart" | "Verilog" | "Zig" | "Bicep" | "Tact" | "Aiken" | "LIGO"
        | "TEAL" => SLASH,

        // Bundled grammars, hash line comment (Vyper and HCL are hash, not
        // slash, per their grammars).
        "Dockerfile" | "DotENV" | "Elixir" | "GraphQL" | "Julia" | "Nix" | "PowerShell"
        | "TOML" | "Vyper" | "HCL" => HASH,

        // Bundled grammars, other line tokens.
        "Clarity" | "FunC" | "Pact" | "WebAssembly" => SEMI2,

        "Lean" | "VHDL" => DASH,

        "Assembly" | "INI" => SEMI,

        // F# has both a // line comment and a (* *) block comment.
        "F#" => CommentStyle { line: Some("//"), block: Some(("(*", "*)")) },

        "Scilla" => PAREN,

        // Common built-in languages (standard tokens).
        "Rust" | "C" | "C++" | "C#" | "Java" | "JavaScript" | "Go" | "PHP" | "Scala"
        | "Objective-C" | "Objective-C++" | "D" | "Groovy" | "Rust Enhanced" => SLASH,

        "Python" | "Ruby" | "Perl" | "R" | "Makefile" | "Tcl" | "CoffeeScript" | "YAML"
        | "Bourne Again Shell (bash)" | "Shell-Unix-Generic" | "Fish" | "PowerShell (Legacy)"
        | "Cargo Manifest" => HASH,

        "Lua" | "Haskell" | "SQL" | "Ada" | "Elm" | "AppleScript" | "Purescript" => DASH,

        "Clojure" | "Lisp" | "Scheme" | "NASM" | "Assembly x86 (NASM)" => SEMI,

        "LaTeX" | "TeX" | "Erlang" | "MATLAB" | "PostScript" | "Bibtex" => PERCENT,

        "HTML" | "XML" | "Markdown" | "HTML (Rails)" | "SVG" | "XSL" => XML,

        "CSS" | "SCSS" | "Sass" | "LESS" => CSS,

        "OCaml" | "OCamlyacc" => PAREN,

        _ => CommentStyle::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Map a `comment.line.<kind>` scope suffix to the token it names.
    fn token_from_scope(scope: &str) -> Option<&'static str> {
        let kind = scope.strip_prefix("comment.line.")?;

        Some(match kind.split('.').next().unwrap_or("") {
            "double-slash" => "//",

            "number-sign" => "#",

            "double-dash" => "--",

            "double-semicolon" => ";;",

            "semicolon" => ";",

            "percentage" | "percent-sign" => "%",

            _ => return None,
        })
    }

    /// Every bundled grammar that names its line comment in a `comment.line.*`
    /// scope must match this table, so the table cannot silently drift from the
    /// grammars it claims to describe.
    #[test]
    fn table_matches_bundled_grammars() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/syntaxes");

        let mut checked = 0;

        let mut mismatches: Vec<String> = Vec::new();

        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();

            if path.extension().and_then(|e| e.to_str()) != Some("sublime-syntax") {
                continue;
            }

            let text = std::fs::read_to_string(&path).unwrap();

            // The grammar's declared name is what syntect keys on at runtime.
            let name = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("name:"))
                .map(|n| n.trim().trim_matches('"').to_string())
                .unwrap_or_default();

            let scope_token = text
                .lines()
                .filter_map(|l| l.trim().strip_prefix("scope:"))
                .filter_map(|s| token_from_scope(s.trim()))
                .next();

            let Some(expected) = scope_token else {
                continue;
            };

            checked += 1;

            if for_syntax(&name).line != Some(expected) {
                mismatches.push(format!(
                    "{name}: grammar says {expected:?}, table says {:?}",
                    for_syntax(&name).line
                ));
            }
        }

        assert!(mismatches.is_empty(), "table drifted from grammars:\n{}", mismatches.join("\n"));

        assert!(checked > 20, "expected to verify many grammars, only saw {checked}");
    }

    #[test]
    fn line_less_languages_fall_back_to_block() {
        assert_eq!(for_syntax("CSS").block, Some(("/*", "*/")));

        assert_eq!(for_syntax("CSS").line, None);

        assert_eq!(for_syntax("HTML").block, Some(("<!--", "-->")));
    }

    #[test]
    fn unknown_language_toggles_nothing() {
        let style = for_syntax("Plain Text");

        assert!(style.line.is_none() && style.block.is_none());
    }
}
