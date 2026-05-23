use std::fs;
use std::path::PathBuf;

use anyhow::Result;

const APP_DIR: &str = "opencode";
const CONFIG_FILE: &str = "config";

/// `$XDG_CONFIG_HOME/opencode` or `$HOME/.config/opencode`.
fn config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join(APP_DIR));
        }
    }

    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|home| PathBuf::from(home).join(".config").join(APP_DIR))
}

/// Folder users drop extra `.tmTheme` files into; loaded at startup.
pub fn themes_dir() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("themes"))
}

/// Folder users drop extra `.sublime-syntax` grammars into; loaded at startup.
pub fn syntaxes_dir() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("syntaxes"))
}

pub fn themes_dir_display() -> String {
    themes_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "~/.config/opencode/themes".to_string())
}

/// The theme name persisted from the last picker selection, if any.
pub fn saved_theme() -> Option<String> {
    let content = fs::read_to_string(config_dir()?.join(CONFIG_FILE)).ok()?;

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
