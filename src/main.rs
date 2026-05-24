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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

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
