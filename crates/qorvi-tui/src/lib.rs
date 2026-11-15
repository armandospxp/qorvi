//! Interfaz de terminal de qorvi.

pub mod action;
pub mod demo;
pub mod explorer;
pub mod keymap;
pub mod state;
pub mod theme;
pub mod update;
pub mod view;

use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;

use crate::action::Action;
use crate::state::AppState;
use crate::update::Effect;

const TICK: Duration = Duration::from_millis(250);

/// Abre la TUI con datos falsos: sin procesos, git ni red.
pub fn run_demo() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, demo::demo_state());
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, mut state: AppState) -> io::Result<()> {
    loop {
        let size = terminal.size()?;
        let tier = view::tier(size.width, size.height);
        state.set_layout(tier == view::Tier::Compact, tier == view::Tier::Full);
        terminal.draw(|frame| view::view(&state, frame))?;

        let action = if event::poll(TICK)? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    keymap::map_key(&state, key)
                }
                Event::Resize(..) => Action::Resize,
                _ => continue,
            }
        } else {
            Action::Tick
        };
        if update::update(&mut state, action).contains(&Effect::Quit) {
            return Ok(());
        }
    }
}
