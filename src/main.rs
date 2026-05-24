mod app;
mod buffer;
mod config;
mod highlight;
mod media;
mod tree;
mod ui;

use std::io::{self, Stdout, Write};
use std::panic;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::{execute, queue};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::App;

type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Recommended Ghostty config for opencode, printed by `ocode --ghostty-config`.
/// Kept here so the setup travels with the binary: reinstall years later, run
/// the flag, and the keys are documented again. Mirrors the README.
// NB: Ghostty has no same-line comments (issue #3350) — every `#` must be on
// its own line, or the rest of the line becomes part of the keybind value.
const GHOSTTY_CONFIG: &str = "\
# opencode — recommended Ghostty config. Append to ~/.config/ghostty/config
# and reload with Cmd+Shift+, (these keys can't be handled inside the app).

# Cmd+Left / Cmd+Right -> line start / end (same as Fn+Left/Right = Home/End).
# Ghostty sends Ctrl+A / Ctrl+E here by default, which collides with Select-all.
keybind = cmd+left=csi:H
keybind = cmd+right=csi:F
keybind = shift+cmd+left=csi:1;2H
keybind = shift+cmd+right=csi:1;2F

# Option+Left / Option+Right -> jump by word.
macos-option-as-alt = true

# Optional: Option+letter as the command key (mirrors opencode's Ctrl keys).
# Option+Left/Right stays word-motion — it rides the arrows, not these letters.
# In order: save, select-all, copy, cut, paste, undo, redo, find, browser,
# reload, quit.
keybind = alt+s=text:\\x13
keybind = alt+a=text:\\x01
keybind = alt+c=text:\\x03
keybind = alt+x=text:\\x18
keybind = alt+v=text:\\x16
keybind = alt+z=text:\\x1a
keybind = alt+y=text:\\x19
keybind = alt+f=text:\\x06
keybind = alt+b=text:\\x02
keybind = alt+r=text:\\x12
keybind = alt+q=text:\\x11
";

/// opencode — a fast terminal code reader & editor.
#[derive(Parser)]
#[command(name = "ocode", version, about)]
struct Cli {
    /// File or directory to open (defaults to the current directory).
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Re-open the style picker to choose and save a different color scheme.
    #[arg(short = 's', long = "style")]
    style: bool,

    /// Print the recommended Ghostty config (keys + inline images) and exit.
    #[arg(long = "ghostty-config")]
    ghostty_config: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.ghostty_config {
        print!("{GHOSTTY_CONFIG}");

        return Ok(());
    }

    if !cli.path.exists() {
        anyhow::bail!("path does not exist: {}", cli.path.display());
    }

    let mut app = App::new(cli.path, cli.style)?;

    let mut terminal = setup_terminal()?;

    let result = run(&mut terminal, &mut app);

    restore_terminal(&mut terminal)?;

    result
}

fn run(terminal: &mut Tui, app: &mut App) -> Result<()> {
    // The cell box of the kitty image currently painted on screen (images are a
    // separate layer from ratatui's cell buffer, so the loop manages them).
    let mut shown: Option<(u16, u16, u16, u16)> = None;

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        if app.should_quit {
            if shown.is_some() {
                clear_image()?;
            }

            return Ok(());
        }

        sync_image(app, &mut shown)?;

        // Block on input, but wake periodically while a file is open (to watch
        // for external changes) or while a flash message is fading.
        match app.wake_after() {
            Some(timeout) => {
                if event::poll(timeout)? {
                    read_key(app)?;
                } else {
                    app.tick();
                }
            }

            None => read_key(app)?,
        }
    }
}

/// Paint or remove the kitty image to match what the renderer asked for. Only
/// acts on a change, so a static image is transmitted once and left alone.
fn sync_image(app: &App, shown: &mut Option<(u16, u16, u16, u16)>) -> Result<()> {
    let want = app.image_placement();

    if want == *shown {
        return Ok(());
    }

    let mut out = io::stdout();

    out.write_all(media::kitty_delete())?;

    if let Some((x, y, cols, rows)) = want {
        queue!(out, MoveTo(x, y))?;

        out.write_all(&app.kitty_image_sequence(cols, rows))?;
    }

    out.flush()?;

    *shown = want;

    Ok(())
}

fn clear_image() -> Result<()> {
    let mut out = io::stdout();

    out.write_all(media::kitty_delete())?;

    out.flush()?;

    Ok(())
}

fn read_key(app: &mut App) -> Result<()> {
    if let Event::Key(key) = event::read()? {
        if key.kind == KeyEventKind::Press {
            app.on_key(key);
        }
    }

    Ok(())
}

fn setup_terminal() -> Result<Tui> {
    install_panic_hook();

    enable_raw_mode().context("enabling raw mode")?;

    let mut stdout = io::stdout();

    execute!(stdout, EnterAlternateScreen).context("entering alternate screen")?;

    let backend = CrosstermBackend::new(stdout);

    Terminal::new(backend).context("creating terminal")
}

fn restore_terminal(terminal: &mut Tui) -> Result<()> {
    disable_raw_mode().context("disabling raw mode")?;

    execute!(terminal.backend_mut(), LeaveAlternateScreen).context("leaving alternate screen")?;

    terminal.show_cursor().context("showing cursor")?;

    Ok(())
}

/// Restore the terminal on panic so a crash never leaves the user in a broken
/// raw-mode screen.
fn install_panic_hook() {
    let hook = panic::take_hook();

    panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();

        let _ = execute!(io::stdout(), LeaveAlternateScreen);

        hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::GHOSTTY_CONFIG;

    #[test]
    fn ghostty_config_carries_the_essential_keys() {
        for needle in [
            "keybind = cmd+left=csi:H",
            "keybind = cmd+right=csi:F",
            "macos-option-as-alt = true",
            "alt+s=text:\\x13",
        ] {
            assert!(GHOSTTY_CONFIG.contains(needle), "missing: {needle}");
        }
    }
}
