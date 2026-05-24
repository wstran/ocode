mod app;
mod buffer;
mod config;
mod highlight;
mod tree;
mod ui;

use std::io::{self, Stdout};
use std::panic;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
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
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        if app.should_quit {
            return Ok(());
        }

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
