use std::fs;
use std::io::Cursor;

use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

fn main() {
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();

    match builder.add_from_folder("assets/syntaxes", true) {
        Ok(()) => println!("added grammars from assets/syntaxes"),

        Err(e) => println!("grammar folder error: {e}"),
    }

    let ss = builder.build();

    println!("=== SYNTAXES: {} ===", ss.syntaxes().len());

    for ext in ["toml", "env", "ini", "cfg", "conf", "Dockerfile", "json", "yaml"] {
        let name = ss.find_syntax_by_extension(ext).map(|s| s.name.clone());

        println!("  {ext:>12} -> {name:?}");
    }

    let ts = ThemeSet::load_defaults();

    let mut ok = 0;

    if let Ok(entries) = fs::read_dir("assets/themes") {
        for path in entries.flatten().map(|e| e.path()) {
            if path.extension().and_then(|e| e.to_str()) == Some("tmTheme") {
                let bytes = fs::read(&path).unwrap();

                if ThemeSet::load_from_reader(&mut Cursor::new(bytes)).is_ok() {
                    ok += 1;
                }
            }
        }
    }

    println!("=== THEMES: {} built-in + {} bundled ===", ts.themes.len(), ok);
}
