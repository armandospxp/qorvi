//! Interfaz de terminal de qorvi.

pub mod action;
pub mod demo;
pub mod explorer;
pub mod keymap;
pub mod layout;
pub mod state;
pub mod theme;
pub mod update;
pub mod view;

use std::io;
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseEventKind,
};
use crossterm::execute;
use ratatui::DefaultTerminal;

use crate::action::Action;
use crate::state::AppState;
use crate::update::Effect;

const TICK: Duration = Duration::from_millis(250);

/// Abre la TUI con datos falsos: sin procesos, git ni red.
pub fn run_demo() -> io::Result<()> {
    let mut terminal = ratatui::init();
    // ratatui::init ya restaura la terminal ante un panic, pero no sabe del ratón: se
    // encadena un hook que lo desactiva antes, para no dejar la shell capturando clics.
    let ratatui_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        ratatui_hook(info);
    }));
    let result = execute!(io::stdout(), EnableMouseCapture)
        .and_then(|()| run(&mut terminal, demo::demo_state()));
    // Se desactiva también si `run` falló, para no dejar la terminal capturando el ratón.
    let _ = execute!(io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, mut state: AppState) -> io::Result<()> {
    loop {
        let size = terminal.size()?;
        let tier = view::tier(size.width, size.height);
        state.size = (size.width, size.height);
        state.set_layout(tier == view::Tier::Compact, tier == view::Tier::Full);
        terminal.draw(|frame| view::view(&state, frame))?;

        let action = if event::poll(TICK)? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    keymap::map_key(&state, key)
                }
                Event::Mouse(m) => match m.kind {
                    // Sin acción: evita redibujar a cada movimiento del ratón.
                    MouseEventKind::Moved | MouseEventKind::Drag(_) => continue,
                    _ => Action::Mouse(m),
                },
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
