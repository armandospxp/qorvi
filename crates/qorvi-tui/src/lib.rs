//! Interfaz de terminal de qorvi.

pub mod action;
pub mod demo;
pub mod detect;
pub mod explorer;
pub mod keymap;
pub mod layout;
pub mod shortcuts;
pub mod state;
pub mod theme;
pub mod update;
pub mod view;

use std::io;
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, KeyboardEnhancementFlags,
    MouseEventKind, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use ratatui::DefaultTerminal;

use crate::action::Action;
use crate::state::{AppState, ToastLevel};
use crate::update::Effect;

const TICK: Duration = Duration::from_millis(250);

/// Pregunta a la terminal si soporta el protocolo de teclado kitty. Requiere el modo raw
/// (la consulta lee la respuesta de la terminal); `None` si la consulta falló.
pub fn probe_keyboard_enhancement() -> Option<bool> {
    crossterm::terminal::supports_keyboard_enhancement().ok()
}

/// Igual, pero activa el modo raw solo durante la consulta (para `qorvi doctor keys`).
/// En Windows no se consulta: la consola entrega todas las teclas.
pub fn probe_keyboard_enhancement_raw() -> io::Result<Option<bool>> {
    if cfg!(windows) {
        return Ok(None);
    }
    // Sin terminal interactiva (redirección, CI) no hay a quién preguntar.
    if crossterm::terminal::enable_raw_mode().is_err() {
        return Ok(None);
    }
    let probe = probe_keyboard_enhancement();
    crossterm::terminal::disable_raw_mode()?;
    Ok(probe)
}

/// Abre la TUI con datos falsos: sin procesos, git ni red.
///
/// `keys_style` es el estilo de atajos pedido (`direct`, `prefix` o `auto`): el de
/// `QORVI_KEYS` o, si no está, el de `[keys] style` en la config.
pub fn run_demo(keys_style: Option<&str>) -> io::Result<()> {
    let mut terminal = ratatui::init();
    // Con el modo raw ya activo se puede consultar a la terminal.
    let enhancement = probe_keyboard_enhancement();
    let env = keys_style;
    let style = detect::resolve_style(env, cfg!(windows), enhancement);
    // Windows entrega todas las teclas por la consola: no necesita (ni admite) el protocolo.
    let push_flags = !cfg!(windows) && enhancement == Some(true);
    if push_flags {
        let _ = execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
    // ratatui::init ya restaura la terminal ante un panic, pero no sabe del ratón ni del
    // protocolo de teclado: se encadena un hook que los desactiva antes, para no dejar la
    // shell capturando clics ni con teclas codificadas.
    let ratatui_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableMouseCapture);
        if push_flags {
            let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        }
        ratatui_hook(info);
    }));
    let mut state = demo::demo_state();
    state.shortcut_style = style;
    if let Some(hint) = detect::startup_hint(env, cfg!(target_os = "macos"), style) {
        state.push_toast(hint, ToastLevel::Info);
    }
    let result =
        execute!(io::stdout(), EnableMouseCapture).and_then(|()| run(&mut terminal, state));
    // Se desactiva también si `run` falló, para no dejar la terminal capturando el ratón.
    let _ = execute!(io::stdout(), DisableMouseCapture);
    if push_flags {
        let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
    }
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
