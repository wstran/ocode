use std::fs;
use std::path::PathBuf;

use anyhow::Result;

const APP_DIR: &str = "ocode";

/// The name this app used before it was renamed. Still read, so an existing
/// install keeps its saved theme and its extra themes and grammars.
const LEGACY_APP_DIR: &str = "opencode";

const CONFIG_FILE: &str = "config";

/// `$XDG_CONFIG_HOME/ocode` or `$HOME/.config/ocode`.
fn config_dir() -> Option<PathBuf> {
    dir_named(APP_DIR)
}

fn legacy_config_dir() -> Option<PathBuf> {
    dir_named(LEGACY_APP_DIR)
}

fn dir_named(name: &str) -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join(name));
        }
    }

    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|home| PathBuf::from(home).join(".config").join(name))
}

/// Folders users drop extra `.tmTheme` files into; loaded at startup. The
/// legacy folder is still read so an existing install keeps working.
pub fn themes_dirs() -> Vec<PathBuf> {
    sub_dirs("themes")
}

/// Folders users drop extra `.sublime-syntax` grammars into; loaded at startup.
pub fn syntaxes_dirs() -> Vec<PathBuf> {
    sub_dirs("syntaxes")
}

fn sub_dirs(leaf: &str) -> Vec<PathBuf> {
    [config_dir(), legacy_config_dir()]
        .into_iter()
        .flatten()
        .map(|dir| dir.join(leaf))
        .collect()
}

pub fn themes_dir_display() -> String {
    config_dir()
        .map(|p| p.join("themes").display().to_string())
        .unwrap_or_else(|| "~/.config/ocode/themes".to_string())
}

/// The theme name persisted from the last picker selection, if any. Falls back
/// to the pre-rename location so an existing choice is not silently lost.
pub fn saved_theme() -> Option<String> {
    [config_dir(), legacy_config_dir()]
        .into_iter()
        .flatten()
        .find_map(|dir| read_theme(&dir.join(CONFIG_FILE)))
}

fn read_theme(path: &PathBuf) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;

    for line in content.lines() {
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "theme" {
                let value = value.trim();

                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }

    None
}

pub fn save_theme(name: &str) -> Result<()> {
    let Some(dir) = config_dir() else {
        return Ok(());
    };

    fs::create_dir_all(&dir)?;

    fs::write(dir.join(CONFIG_FILE), format!("theme = {name}\n"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The app was renamed from opencode to ocode. An install that already had
    /// a theme saved under the old name must keep it rather than being sent
    /// back to the picker, so the old location stays readable.
    #[test]
    fn a_theme_saved_under_the_old_name_is_still_found() {
        let root = std::env::temp_dir().join(format!("ocode_cfg_{}", std::process::id()));

        let _ = fs::remove_dir_all(&root);

        let legacy = root.join(LEGACY_APP_DIR);

        fs::create_dir_all(&legacy).unwrap();

        fs::write(legacy.join(CONFIG_FILE), "theme = Dracula\n").unwrap();

        // SAFETY: single-threaded test, restored before it returns.
        let previous = std::env::var("XDG_CONFIG_HOME").ok();

        unsafe { std::env::set_var("XDG_CONFIG_HOME", &root) };

        assert_eq!(saved_theme().as_deref(), Some("Dracula"));

        // A theme saved now goes to the new location and wins.
        save_theme("One Dark").unwrap();

        assert!(root.join(APP_DIR).join(CONFIG_FILE).exists(), "written under the new name");

        assert_eq!(saved_theme().as_deref(), Some("One Dark"));

        match previous {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },

            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }

        let _ = fs::remove_dir_all(&root);
    }
}
