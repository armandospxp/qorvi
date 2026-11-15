//! Atajos globales: se comparan antes de enviar la tecla al panel con foco.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;
use crate::state::{AgentStatus, AppState};

/// ¿Es la tecla prefijo? Fijada a `Ctrl+Space` en la demo; algunos terminales envían
/// NUL, que crossterm da como `Ctrl+@` o `Ctrl+2`. Toda la comparación vive aquí para
/// poder cambiarla cuando el prefijo salga de la configuración.
fn is_prefix(key: &KeyEvent) -> bool {
    key.modifiers == KeyModifiers::CONTROL && matches!(key.code, KeyCode::Char(' ' | '@' | '2'))
}

/// Tecla de un atajo global (la misma con `Alt+` y tras el prefijo).
fn shortcut(c: char) -> Option<Action> {
    Some(match c {
        'h' => Action::FocusLeft,
        'j' => Action::FocusDown,
        'k' => Action::FocusUp,
        'l' => Action::FocusRight,
        '1'..='9' => Action::SelectTask(c as usize - '1' as usize),
        'n' => Action::NewTask,
        'd' => Action::OpenReview,
        'z' => Action::ToggleZoom,
        't' => Action::ToggleTerminal,
        'e' => Action::ToggleExplorer,
        'm' => Action::ShowLastToast,
        'q' => Action::Quit,
        '?' => Action::Help,
        _ => return None,
    })
}

/// Nombre legible de una tecla para el aviso de atajo desconocido.
fn key_name(key: &KeyEvent) -> String {
    let base = match key.code {
        KeyCode::Char(c) => c.to_string(),
        KeyCode::F(n) => format!("F{n}"),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Tab | KeyCode::BackTab => "Tab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Up => "↑".to_string(),
        KeyCode::Down => "↓".to_string(),
        KeyCode::Left => "←".to_string(),
        KeyCode::Right => "→".to_string(),
        other => format!("{other:?}"),
    };
    let mut name = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        name.push_str("Ctrl+");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        name.push_str("Alt+");
    }
    name + &base
}

pub fn map_key(state: &AppState, key: KeyEvent) -> Action {
    if state.prefix_pending {
        // Todo lo que llega con el prefijo pendiente lo resuelve (o lo cancela).
        if is_prefix(&key) {
            return Action::PrefixLiteral;
        }
        if key.code == KeyCode::Esc {
            return Action::PrefixCancel;
        }
        if let KeyCode::Char(c) = key.code {
            if (key.modifiers - KeyModifiers::SHIFT).is_empty() {
                if let Some(action) = shortcut(c) {
                    return action;
                }
            }
        }
        return Action::PrefixUnknown(key_name(&key));
    }
    if is_prefix(&key) {
        return Action::PrefixStart;
    }
    if (key.modifiers - KeyModifiers::SHIFT) == KeyModifiers::ALT {
        if let Some(action) = match key.code {
            KeyCode::Char(c) => shortcut(c),
            _ => None,
        } {
            return action;
        }
    }
    if key.modifiers == KeyModifiers::NONE {
        match key.code {
            KeyCode::F(1) => return Action::Help,
            KeyCode::F(5) => return Action::DemoSetStatus(AgentStatus::Working),
            KeyCode::F(6) => return Action::DemoSetStatus(AgentStatus::NeedsPermission),
            KeyCode::F(7) => return Action::DemoSetStatus(AgentStatus::Done),
            KeyCode::F(8) => return Action::DemoSetStatus(AgentStatus::Error),
            _ => {}
        }
    }
    Action::Key(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_state;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn alt_digit_selects_task() {
        let s = demo_state();
        assert_eq!(
            map_key(&s, key(KeyCode::Char('1'), KeyModifiers::ALT)),
            Action::SelectTask(0)
        );
        assert_eq!(
            map_key(&s, key(KeyCode::Char('9'), KeyModifiers::ALT)),
            Action::SelectTask(8)
        );
    }

    #[test]
    fn alt_e_toggles_explorer() {
        let s = demo_state();
        assert_eq!(
            map_key(&s, key(KeyCode::Char('e'), KeyModifiers::ALT)),
            Action::ToggleExplorer
        );
        let plain = key(KeyCode::Char('e'), KeyModifiers::NONE);
        assert_eq!(map_key(&s, plain), Action::Key(plain));
    }

    #[test]
    fn alt_q_quits_plain_q_is_key() {
        let s = demo_state();
        assert_eq!(
            map_key(&s, key(KeyCode::Char('q'), KeyModifiers::ALT)),
            Action::Quit
        );
        let plain = key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(map_key(&s, plain), Action::Key(plain));
    }

    #[test]
    fn alt_enter_is_not_global() {
        let s = demo_state();
        let k = key(KeyCode::Enter, KeyModifiers::ALT);
        assert_eq!(map_key(&s, k), Action::Key(k));
    }

    #[test]
    fn function_keys() {
        let s = demo_state();
        assert_eq!(
            map_key(&s, key(KeyCode::F(5), KeyModifiers::NONE)),
            Action::DemoSetStatus(AgentStatus::Working)
        );
        assert_eq!(
            map_key(&s, key(KeyCode::F(1), KeyModifiers::NONE)),
            Action::Help
        );
    }

    fn ctrl_space() -> KeyEvent {
        key(KeyCode::Char(' '), KeyModifiers::CONTROL)
    }

    fn pending() -> AppState {
        let mut s = demo_state();
        s.prefix_pending = true;
        s
    }

    fn plain(c: char) -> KeyEvent {
        key(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn prefix_variants_start_the_prefix() {
        let s = demo_state();
        for code in [' ', '@', '2'] {
            let k = key(KeyCode::Char(code), KeyModifiers::CONTROL);
            assert_eq!(map_key(&s, k), Action::PrefixStart, "{code:?}");
        }
        // Sin Ctrl no es prefijo.
        assert_eq!(map_key(&s, plain(' ')), Action::Key(plain(' ')));
    }

    #[test]
    fn prefix_then_table_key() {
        let s = pending();
        assert_eq!(map_key(&s, plain('q')), Action::Quit);
        assert_eq!(map_key(&s, plain('2')), Action::SelectTask(1));
        assert_eq!(map_key(&s, plain('h')), Action::FocusLeft);
        assert_eq!(map_key(&s, plain('?')), Action::Help);
        assert_eq!(
            map_key(&s, key(KeyCode::Char('?'), KeyModifiers::SHIFT)),
            Action::Help
        );
    }

    #[test]
    fn prefix_esc_twice_and_unknown() {
        let s = pending();
        assert_eq!(
            map_key(&s, key(KeyCode::Esc, KeyModifiers::NONE)),
            Action::PrefixCancel
        );
        assert_eq!(map_key(&s, ctrl_space()), Action::PrefixLiteral);
        assert_eq!(
            map_key(&s, key(KeyCode::Char('@'), KeyModifiers::CONTROL)),
            Action::PrefixLiteral
        );
        assert_eq!(map_key(&s, plain('x')), Action::PrefixUnknown("x".into()));
        // `r` no es global: el explorador la usa para refrescar.
        assert_eq!(map_key(&s, plain('r')), Action::PrefixUnknown("r".into()));
        assert_eq!(
            map_key(&s, key(KeyCode::Enter, KeyModifiers::NONE)),
            Action::PrefixUnknown("Enter".into())
        );
    }

    #[test]
    fn alt_shortcuts_work_while_prefix_is_pending_too_but_never_leak_keys() {
        let s = pending();
        // Con el prefijo pendiente solo cuenta la tabla: Alt+q es una tecla desconocida.
        assert_eq!(
            map_key(&s, key(KeyCode::Char('q'), KeyModifiers::ALT)),
            Action::PrefixUnknown("Alt+q".into())
        );
    }

    #[test]
    fn alt_help_and_text_keys() {
        let s = demo_state();
        assert_eq!(
            map_key(&s, key(KeyCode::Char('?'), KeyModifiers::ALT)),
            Action::Help
        );
        assert_eq!(map_key(&s, plain('?')), Action::Key(plain('?')));
    }
}
