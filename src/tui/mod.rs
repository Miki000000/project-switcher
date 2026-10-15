mod app;
mod theme;
mod view;

use std::io::{Stderr, stderr};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

pub use app::View;
use app::{App, Outcome};

use crate::config::Config;

pub fn run(cfg: Config, view: View) -> Result<()> {
    let mut app = App::new(cfg, view);
    let mut terminal = setup()?;
    let result = event_loop(&mut terminal, &mut app);
    restore();
    match result? {
        Outcome::Cd(path) => crate::shell::request_cd(&path),
        Outcome::Quit => Ok(()),
    }
}

fn event_loop(terminal: &mut Terminal<CrosstermBackend<Stderr>>, app: &mut App) -> Result<Outcome> {
    loop {
        terminal.draw(|f| view::render(f, app))?;
        let timeout = Duration::from_millis(if app.animating() {
            16
        } else if app.busy() {
            80
        } else {
            250
        });
        if event::poll(timeout)? {
            loop {
                match event::read()? {
                    Event::Key(key) => app.on_key(key),
                    Event::Paste(text) => app.on_paste(&text),
                    _ => {}
                }
                if app.outcome.is_some() || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
        app.drain();
        if let Some(outcome) = app.outcome.take() {
            return Ok(outcome);
        }
    }
}

fn setup() -> Result<Terminal<CrosstermBackend<Stderr>>> {
    enable_raw_mode()?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        hook(info);
    }));
    execute!(stderr(), EnterAlternateScreen, EnableBracketedPaste)?;
    Ok(Terminal::new(CrosstermBackend::new(stderr()))?)
}

fn restore() {
    let _ = execute!(
        stderr(),
        DisableBracketedPaste,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    );
    let _ = disable_raw_mode();
}
