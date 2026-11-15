//! Atajos globales: se comparan antes de enviar la tecla al panel con foco.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;
use crate::state::{AgentStatus, AppState};

pub fn map_key(_state: &AppState, key: KeyEvent) -> Action {
    if key.modifiers == KeyModifiers::ALT {
        if let KeyCode::Char(c) = key.code {
            match c {
                'h' => return Action::FocusLeft,
                'j' => return Action::FocusDown,
                'k' => return Action::FocusUp,
                'l' => return Action::FocusRight,
                '1'..='9' => return Action::SelectTask(c as usize - '1' as usize),
                'n' => return Action::NewTask,
                'd' => return Action::OpenReview,
                'z' => return Action::ToggleZoom,
                't' => return Action::ToggleTerminal,
                'm' => return Action::ShowLastToast,
                'q' => return Action::Quit,
                _ => {}
            }
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
}
