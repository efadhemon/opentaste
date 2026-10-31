//! `taste ui`: the terminal dashboard.
//!
//! State is in [`app`] and drawing in [`ui`]; this module only owns the
//! terminal and turns its events into [`app::Key`]s. `ratatui::try_init`
//! installs a panic hook that restores the terminal (leaves raw mode and the
//! alternate screen) and then chains to the previous hook, so a panic still
//! prints its message and leaves a usable shell. The release profile's
//! `panic = "abort"` runs the hook before aborting.

pub mod app;
pub mod highlight;
pub mod ui;

use anyhow::Context;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use taste_core::RuleStore;

use crate::output::glyphs;
use app::{App, Key};

pub fn run(store: &RuleStore) -> anyhow::Result<()> {
    let glyphs = glyphs();
    let mut app = App::load(store)?;
    let mut terminal = ratatui::try_init().context("opening the terminal")?;
    let result = (|| -> anyhow::Result<()> {
        while !app.quit {
            terminal.draw(|frame| ui::draw(frame, &app, &glyphs))?;
            let Event::Key(press) = event::read()? else {
                continue;
            };
            // Windows reports releases as well as presses.
            if press.kind != KeyEventKind::Press {
                continue;
            }
            #[cfg(debug_assertions)]
            if press.code == KeyCode::Char('!') && std::env::var_os("TASTE_UI_PANIC").is_some() {
                panic!("deliberate panic to check that the terminal is restored");
            }
            if let Some(key) = translate(press.code) {
                app.on_key(key, store);
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}

fn translate(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::Enter => Key::Enter,
        KeyCode::Esc => Key::Esc,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Tab | KeyCode::BackTab => Key::Tab,
        KeyCode::Char(c) => Key::Char(c),
        _ => return None,
    })
}
