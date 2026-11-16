//! Atajos globales: se comparan antes de enviar la tecla al panel con foco.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;
use crate::shortcuts::{direct_action, prefix_action};
use crate::state::{AgentStatus, AppState};

/// ¿Es la tecla prefijo? Fijada a `Ctrl+Space` en la demo; algunos terminales envían
/// NUL, que crossterm da como `Ctrl+@` o `Ctrl+2`. Toda la comparación vive aquí para
/// poder cambiarla cuando el prefijo salga de la configuración.
/// Con `Ctrl` como modificador directo, `Ctrl+2` es "ir a la tarea 2", no un prefijo.
fn is_prefix(state: &AppState, key: &KeyEvent) -> bool {
    let two = state.direct_mod != KeyModifiers::CONTROL;
    key.modifiers == KeyModifiers::CONTROL
        && match key.code {
            KeyCode::Char(' ' | '@') => true,
            KeyCode::Char('2') => two,
            _ => false,
        }
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
    if key.modifiers.contains(KeyModifiers::SUPER) {
        name.push_str("Cmd+");
    }
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
        if is_prefix(state, &key) {
            return Action::PrefixLiteral;
        }
        if key.code == KeyCode::Esc {
            return Action::PrefixCancel;
        }
        if let KeyCode::Char(c) = key.code {
            if (key.modifiers - KeyModifiers::SHIFT).is_empty() {
                if let Some(action) = prefix_action(c) {
                    return action;
                }
            }
        }
        return Action::PrefixUnknown(key_name(&key));
    }
    if is_prefix(state, &key) {
        return Action::PrefixStart;
    }
    if (key.modifiers - KeyModifiers::SHIFT) == KeyModifiers::ALT {
        if let Some(action) = match key.code {
            KeyCode::Char(c) => prefix_action(c),
            _ => None,
        } {
            return action;
        }
    }
    if let Some(action) = direct_action(state.direct_mod, &key) {
        return action;
    }
    if key.modifiers == KeyModifiers::NONE {
        match key.code {
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
        let mut s = demo_state();
        s.direct_mod = KeyModifiers::SUPER;
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

    const BOTH_MODS: [KeyModifiers; 2] = [KeyModifiers::SUPER, KeyModifiers::CONTROL];

    fn with_mod(direct_mod: KeyModifiers) -> AppState {
        let mut s = demo_state();
        s.direct_mod = direct_mod;
        s
    }

    #[test]
    fn direct_shortcuts_on_both_modifiers() {
        for m in BOTH_MODS {
            let s = with_mod(m);
            let ma = m | KeyModifiers::ALT;
            let cases = [
                (KeyCode::Left, ma, Action::FocusLeft),
                (KeyCode::Down, ma, Action::FocusDown),
                (KeyCode::Up, ma, Action::FocusUp),
                (KeyCode::Right, ma, Action::FocusRight),
                (KeyCode::Char('b'), m, Action::ToggleExplorer),
                (KeyCode::Char('j'), m, Action::ToggleTerminal),
                (KeyCode::Char('n'), ma, Action::NewTask),
                (KeyCode::Char('r'), ma, Action::OpenReview),
                (KeyCode::Char('q'), ma, Action::Quit),
                (KeyCode::F(1), KeyModifiers::NONE, Action::Help),
            ];
            for (code, mods, want) in cases {
                assert_eq!(map_key(&s, key(code, mods)), want, "{code:?} {mods:?}");
            }
            for n in 1..=9u8 {
                let c = (b'0' + n) as char;
                assert_eq!(
                    map_key(&s, key(KeyCode::Char(c), m)),
                    Action::SelectTask(usize::from(n) - 1),
                    "{m:?} {c}"
                );
            }
        }
    }

    #[test]
    fn direct_letters_accept_uppercase() {
        for m in BOTH_MODS {
            let s = with_mod(m);
            assert_eq!(
                map_key(&s, key(KeyCode::Char('B'), m | KeyModifiers::SHIFT)),
                Action::ToggleExplorer
            );
            assert_eq!(
                map_key(&s, key(KeyCode::Char('B'), m)),
                Action::ToggleExplorer
            );
            assert_eq!(
                map_key(
                    &s,
                    key(
                        KeyCode::Char('Q'),
                        m | KeyModifiers::ALT | KeyModifiers::SHIFT
                    )
                ),
                Action::Quit
            );
        }
    }

    #[test]
    fn direct_matches_modifiers_exactly() {
        for m in BOTH_MODS {
            let s = with_mod(m);
            // Falta el modificador, falta Alt o sobra Alt: no es el atajo.
            for k in [
                key(KeyCode::Char('b'), KeyModifiers::NONE),
                key(KeyCode::Char('b'), m | KeyModifiers::ALT),
                key(KeyCode::Char('n'), m),
                key(KeyCode::Char('q'), m),
                key(KeyCode::Left, m),
                key(KeyCode::Char('3'), m | KeyModifiers::ALT),
                key(KeyCode::Char('3'), m | KeyModifiers::SHIFT),
            ] {
                assert_eq!(map_key(&s, k), Action::Key(k), "{k:?}");
            }
        }
        // Con Cmd, un Ctrl+B sigue siendo del panel, y al revés.
        let mac = with_mod(KeyModifiers::SUPER);
        let k = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(map_key(&mac, k), Action::Key(k));
        let other = with_mod(KeyModifiers::CONTROL);
        let k = key(KeyCode::Char('b'), KeyModifiers::SUPER);
        assert_eq!(map_key(&other, k), Action::Key(k));
    }

    #[test]
    fn legacy_ctrl_encodings_are_direct_shortcuts() {
        // Lo que crossterm entrega en Linux/Unix sin protocolo kitty (modo raw):
        // 0x02 -> Ctrl+b, 0x0A -> Ctrl+j, ESC 0x0E -> Ctrl+Alt+n, CSI 1;7D -> Ctrl+Alt+Left.
        let s = with_mod(KeyModifiers::CONTROL);
        let ca = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(
            map_key(&s, key(KeyCode::Char('b'), KeyModifiers::CONTROL)),
            Action::ToggleExplorer
        );
        assert_eq!(
            map_key(&s, key(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            Action::ToggleTerminal
        );
        assert_eq!(map_key(&s, key(KeyCode::Char('n'), ca)), Action::NewTask);
        assert_eq!(map_key(&s, key(KeyCode::Left, ca)), Action::FocusLeft);
    }

    #[test]
    fn ctrl_digit_two_is_a_task_with_ctrl_and_a_prefix_with_cmd() {
        let ctrl = with_mod(KeyModifiers::CONTROL);
        let k = key(KeyCode::Char('2'), KeyModifiers::CONTROL);
        assert_eq!(map_key(&ctrl, k), Action::SelectTask(1));
        // Ctrl+Space y Ctrl+@ siguen siendo el prefijo.
        assert_eq!(map_key(&ctrl, ctrl_space()), Action::PrefixStart);
        let mac = with_mod(KeyModifiers::SUPER);
        assert_eq!(map_key(&mac, k), Action::PrefixStart);
    }

    #[test]
    fn plain_ctrl_keys_still_reach_the_pane() {
        for m in BOTH_MODS {
            let s = with_mod(m);
            for c in ['c', 'l', 'a', 'e', 'k', 'd'] {
                let k = key(KeyCode::Char(c), KeyModifiers::CONTROL);
                assert_eq!(map_key(&s, k), Action::Key(k), "Ctrl+{c} con {m:?}");
            }
        }
    }

    #[test]
    fn prefix_and_alt_work_with_either_direct_modifier() {
        for m in BOTH_MODS {
            let s = with_mod(m);
            assert_eq!(map_key(&s, ctrl_space()), Action::PrefixStart);
            assert_eq!(
                map_key(&s, key(KeyCode::Char('d'), KeyModifiers::ALT)),
                Action::OpenReview
            );
            let mut p = with_mod(m);
            p.prefix_pending = true;
            assert_eq!(map_key(&p, plain('d')), Action::OpenReview);
            assert_eq!(map_key(&p, plain('z')), Action::ToggleZoom);
        }
    }

    #[test]
    fn quit_combo_quits_with_any_modal_open() {
        use crate::state::Modal;
        use crate::update::{update, Effect};
        for m in BOTH_MODS {
            for modal in [
                Modal::Help,
                Modal::NewTask {
                    name: String::new(),
                    agent_idx: 0,
                    base: "main".to_string(),
                    field: 0,
                },
                Modal::Confirm {
                    message: "x".to_string(),
                    action: crate::state::ConfirmAction::DiscardTask(0),
                },
                Modal::Review {
                    files: crate::demo::fake_changes(),
                    selected: 0,
                },
            ] {
                let mut s = with_mod(m);
                s.modal = modal;
                let action = map_key(&s, key(KeyCode::Char('q'), m | KeyModifiers::ALT));
                assert_eq!(action, Action::Quit);
                assert_eq!(update(&mut s, action), vec![Effect::Quit]);
            }
        }
    }

    #[test]
    fn unknown_prefix_key_names_cmd() {
        let s = pending();
        assert_eq!(
            map_key(&s, key(KeyCode::Char('b'), KeyModifiers::SUPER)),
            Action::PrefixUnknown("Cmd+b".into())
        );
    }
}
