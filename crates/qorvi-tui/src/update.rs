//! `update`: transforma el estado según una acción. No hace IO.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;
use crate::demo::fake_changes;
use crate::state::{AgentStatus, AppState, ChatItem, ConfirmAction, Modal, Pane, Task, ToastLevel};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Quit,
}

pub fn update(state: &mut AppState, action: Action) -> Vec<Effect> {
    let modal_open = state.modal != Modal::None;
    match action {
        Action::Quit => {
            state.should_quit = true;
            return vec![Effect::Quit];
        }
        Action::Tick => tick(state),
        Action::Resize => {}
        Action::ShowLastToast => {
            if let Some(last) = state.last_toast.clone() {
                state.toasts.retain(|t| t.text != last.text);
                state.push_toast(last.text, last.level);
            }
        }
        Action::Help => match state.modal {
            Modal::None => state.modal = Modal::Help,
            Modal::Help => state.modal = Modal::None,
            _ => {}
        },
        Action::DemoSetStatus(status) => set_status(state, status),
        Action::Key(key) => handle_key(state, key),
        // El resto de acciones de navegación no actúan con un diálogo abierto.
        _ if modal_open => {}
        Action::FocusLeft => focus_horizontal(state, false),
        Action::FocusRight => focus_horizontal(state, true),
        Action::FocusUp => {
            if state.focus == Pane::Terminal {
                state.focus = Pane::Editor;
            }
        }
        Action::FocusDown => state.focus = Pane::Terminal,
        Action::SelectTask(i) => select_task(state, i),
        Action::NewTask => open_new_task(state),
        Action::OpenReview => {
            if state.active_task().is_some() {
                state.modal = Modal::Review {
                    files: fake_changes(),
                    selected: 0,
                };
            }
        }
        Action::ToggleZoom => state.zoom = !state.zoom,
        Action::ToggleTerminal => state.terminal_minimized = !state.terminal_minimized,
    }
    Vec::new()
}

fn tick(state: &mut AppState) {
    for t in &mut state.toasts {
        t.ttl_ticks = t.ttl_ticks.saturating_sub(1);
    }
    state.toasts.retain(|t| t.ttl_ticks > 0);
}

fn focus_horizontal(state: &mut AppState, right: bool) {
    const ORDER: [Pane; 3] = [Pane::Editor, Pane::Agent, Pane::Terminal];
    if state.compact {
        let pos = ORDER.iter().position(|p| *p == state.focus).unwrap_or(0);
        let next = if right {
            pos + 1
        } else {
            pos + ORDER.len() - 1
        };
        state.focus = ORDER[next % ORDER.len()];
        return;
    }
    state.focus = match (state.focus, right) {
        (Pane::Editor, true) => Pane::Agent,
        (Pane::Agent, false) => Pane::Editor,
        (other, _) => other,
    };
}

fn select_task(state: &mut AppState, i: usize) {
    if i >= state.tasks.len() {
        return;
    }
    state.active = i;
    if state.tasks[i].status == AgentStatus::Done {
        state.tasks[i].status = AgentStatus::Idle;
    }
}

fn open_new_task(state: &mut AppState) {
    state.modal = Modal::NewTask {
        name: String::new(),
        agent_idx: 0,
        base: "main".to_string(),
        field: 0,
    };
}

fn set_status(state: &mut AppState, status: AgentStatus) {
    let Some(task) = state.active_task_mut() else {
        return;
    };
    task.status = status;
    let name = task.name.clone();
    match status {
        AgentStatus::NeedsPermission => {
            task.chat.push(ChatItem::Permission {
                title: "Ejecutar: cargo test".to_string(),
                options: vec![
                    "Permitir una vez".to_string(),
                    "Permitir siempre".to_string(),
                    "Rechazar".to_string(),
                ],
                selected: 0,
                resolved: None,
            });
            state.push_toast(
                format!("Tarea {name} esperando permiso"),
                ToastLevel::Warning,
            );
        }
        AgentStatus::Error => {
            state.push_toast(
                format!("El agente de {name} falló (simulado)"),
                ToastLevel::Error,
            );
        }
        _ => {}
    }
}

fn is_text_modifier(m: KeyModifiers) -> bool {
    (m - KeyModifiers::SHIFT).is_empty()
}

fn handle_key(state: &mut AppState, key: KeyEvent) {
    match state.modal {
        Modal::None => {
            if state.focus == Pane::Agent {
                agent_key(state, key);
            }
            // Editor y Terminal: en la demo las teclas se ignoran.
        }
        Modal::NewTask { .. } => new_task_key(state, key),
        Modal::Confirm { .. } => confirm_key(state, key),
        Modal::Review { .. } => review_key(state, key),
        Modal::Help => {
            if key.code == KeyCode::Esc {
                state.modal = Modal::None;
            }
        }
    }
}

fn last_unresolved_permission(task: &Task) -> Option<usize> {
    task.chat
        .iter()
        .rposition(|c| matches!(c, ChatItem::Permission { resolved: None, .. }))
}

fn agent_key(state: &mut AppState, key: KeyEvent) {
    let Some(task) = state.active_task_mut() else {
        return;
    };
    let perm_idx = last_unresolved_permission(task);

    match key.code {
        KeyCode::Esc => {
            if task.status == AgentStatus::Working {
                task.status = AgentStatus::Idle;
                task.chat
                    .push(ChatItem::Notice("Turno cancelado".to_string()));
            }
        }
        KeyCode::Up | KeyCode::Down if perm_idx.is_some() => {
            if let Some(ChatItem::Permission {
                options, selected, ..
            }) = perm_idx.and_then(|i| task.chat.get_mut(i))
            {
                let n = options.len();
                if n > 0 {
                    *selected = if key.code == KeyCode::Up {
                        (*selected + n - 1) % n
                    } else {
                        (*selected + 1) % n
                    };
                }
            }
        }
        KeyCode::Enter
            if key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT) =>
        {
            task.input.push('\n');
        }
        KeyCode::Enter => {
            if let Some(i) = perm_idx {
                let mut answered = None;
                if let Some(ChatItem::Permission {
                    options,
                    selected,
                    resolved,
                    ..
                }) = task.chat.get_mut(i)
                {
                    *resolved = Some(*selected);
                    answered = options.get(*selected).cloned();
                }
                task.status = AgentStatus::Working;
                if let Some(option) = answered {
                    state.push_toast(format!("Permiso respondido: {option}"), ToastLevel::Info);
                }
            } else if !task.input.trim().is_empty() {
                let text = std::mem::take(&mut task.input);
                task.chat.push(ChatItem::User(text));
                task.chat
                    .push(ChatItem::Agent("DEMO: respuesta simulada.".to_string()));
            }
        }
        KeyCode::Backspace => {
            task.input.pop();
        }
        KeyCode::Char(c) if is_text_modifier(key.modifiers) => task.input.push(c),
        _ => {}
    }
}

fn new_task_key(state: &mut AppState, key: KeyEvent) {
    let n_agents = state.agents.len().max(1);
    let Modal::NewTask {
        name,
        agent_idx,
        base,
        field,
    } = &mut state.modal
    else {
        return;
    };
    match key.code {
        KeyCode::Esc => state.modal = Modal::None,
        KeyCode::Tab => *field = (*field + 1) % 3,
        KeyCode::BackTab => *field = (*field + 2) % 3,
        KeyCode::Left if *field == 1 => *agent_idx = (*agent_idx + n_agents - 1) % n_agents,
        KeyCode::Right if *field == 1 => *agent_idx = (*agent_idx + 1) % n_agents,
        KeyCode::Backspace => match *field {
            0 => {
                name.pop();
            }
            2 => {
                base.pop();
            }
            _ => {}
        },
        KeyCode::Char(c) if is_text_modifier(key.modifiers) => match *field {
            0 => name.push(c),
            2 => base.push(c),
            _ => {}
        },
        KeyCode::Enter => {
            let (name, idx, base) = (name.clone(), *agent_idx, base.clone());
            create_task(state, &name, idx, &base);
        }
        _ => {}
    }
}

fn create_task(state: &mut AppState, name: &str, agent_idx: usize, base: &str) {
    let error = if name.is_empty() {
        Some("El nombre de la tarea no puede estar vacío")
    } else if name.chars().any(|c| c.is_whitespace() || c == '/') {
        Some("El nombre no puede contener espacios ni '/'")
    } else if state.tasks.iter().any(|t| t.name == name) {
        Some("Ya existe una tarea con ese nombre")
    } else if base.trim().is_empty() {
        Some("La rama base no puede estar vacía")
    } else {
        None
    };
    if let Some(msg) = error {
        state.push_toast(msg, ToastLevel::Error);
        return;
    }
    let agent = state
        .agents
        .get(agent_idx)
        .cloned()
        .unwrap_or_else(|| "claude".to_string());
    let mut task = Task::new(name, &agent, base.trim());
    task.chat.push(ChatItem::Notice(
        "DEMO: tarea simulada, no se creó ningún worktree".to_string(),
    ));
    state.tasks.push(task);
    state.active = state.tasks.len() - 1;
    state.focus = Pane::Agent;
    state.modal = Modal::None;
    state.push_toast(format!("Tarea {name} creada (DEMO)"), ToastLevel::Info);
}

fn review_key(state: &mut AppState, key: KeyEvent) {
    let active = state.active;
    let task_name = state.active_task().map(|t| t.name.clone());
    let Modal::Review { files, selected } = &mut state.modal else {
        return;
    };
    let action_name = match key.code {
        KeyCode::Esc => {
            state.modal = Modal::None;
            return;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if *selected + 1 < files.len() {
                *selected += 1;
            }
            return;
        }
        KeyCode::Char('k') | KeyCode::Up => {
            *selected = selected.saturating_sub(1);
            return;
        }
        KeyCode::Char('c') => "commit",
        KeyCode::Char('m') => "merge",
        KeyCode::Char('p') => "push + PR",
        KeyCode::Char('x') => {
            let name = task_name.unwrap_or_default();
            state.modal = Modal::Confirm {
                message: format!("¿Descartar la tarea {name}? Se borrarán su worktree y su rama."),
                action: ConfirmAction::DiscardTask(active),
            };
            return;
        }
        _ => return,
    };
    state.push_toast(
        format!("DEMO: no se ejecutó {action_name}"),
        ToastLevel::Info,
    );
}

fn confirm_key(state: &mut AppState, key: KeyEvent) {
    let Modal::Confirm { action, .. } = state.modal else {
        return;
    };
    match key.code {
        KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Enter => {
            state.modal = Modal::None;
            match action {
                ConfirmAction::DiscardTask(idx) => discard_task(state, idx),
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => state.modal = Modal::None,
        _ => {}
    }
}

fn discard_task(state: &mut AppState, idx: usize) {
    if idx >= state.tasks.len() {
        return;
    }
    let removed = state.tasks.remove(idx);
    state.push_toast(
        format!("Tarea {} descartada (DEMO)", removed.name),
        ToastLevel::Info,
    );
    if state.tasks.is_empty() {
        state.active = 0;
        open_new_task(state);
        return;
    }
    if idx < state.active {
        state.active -= 1;
    }
    state.active = state.active.min(state.tasks.len() - 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_state;
    use crate::state::{Toast, TOAST_TTL_TICKS};

    fn press(state: &mut AppState, code: KeyCode) {
        update(state, Action::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    fn type_text(state: &mut AppState, text: &str) {
        for c in text.chars() {
            press(state, KeyCode::Char(c));
        }
    }

    #[test]
    fn focus_moves_between_panes() {
        let mut s = demo_state();
        s.focus = Pane::Editor;
        update(&mut s, Action::FocusRight);
        assert_eq!(s.focus, Pane::Agent);
        update(&mut s, Action::FocusDown);
        assert_eq!(s.focus, Pane::Terminal);
        update(&mut s, Action::FocusRight);
        assert_eq!(s.focus, Pane::Terminal);
        update(&mut s, Action::FocusUp);
        assert_eq!(s.focus, Pane::Editor);
        update(&mut s, Action::FocusLeft);
        assert_eq!(s.focus, Pane::Editor);
    }

    #[test]
    fn compact_focus_cycles() {
        let mut s = demo_state();
        s.compact = true;
        s.focus = Pane::Editor;
        update(&mut s, Action::FocusRight);
        update(&mut s, Action::FocusRight);
        assert_eq!(s.focus, Pane::Terminal);
        update(&mut s, Action::FocusRight);
        assert_eq!(s.focus, Pane::Editor);
        update(&mut s, Action::FocusLeft);
        assert_eq!(s.focus, Pane::Terminal);
    }

    #[test]
    fn select_task_out_of_range_is_ignored() {
        let mut s = demo_state();
        update(&mut s, Action::SelectTask(7));
        assert_eq!(s.active, 0);
        update(&mut s, Action::SelectTask(1));
        assert_eq!(s.active, 1);
    }

    #[test]
    fn selecting_done_task_sets_idle() {
        let mut s = demo_state();
        assert_eq!(s.tasks[2].status, AgentStatus::Done);
        update(&mut s, Action::SelectTask(2));
        assert_eq!(s.tasks[2].status, AgentStatus::Idle);
    }

    fn submit_new_task(s: &mut AppState, name: &str) {
        update(s, Action::NewTask);
        type_text(s, name);
        press(s, KeyCode::Enter);
    }

    #[test]
    fn new_task_rejects_invalid_names() {
        for bad in ["", "a b", "a/b", "APP-12"] {
            let mut s = demo_state();
            submit_new_task(&mut s, bad);
            assert_eq!(s.tasks.len(), 3, "nombre {bad:?}");
            assert!(matches!(s.modal, Modal::NewTask { .. }));
            assert_eq!(s.toasts.last().map(|t| t.level), Some(ToastLevel::Error));
        }
    }

    #[test]
    fn new_task_valid_is_created_and_activated() {
        let mut s = demo_state();
        s.focus = Pane::Editor;
        update(&mut s, Action::NewTask);
        press(&mut s, KeyCode::Tab);
        press(&mut s, KeyCode::Right);
        press(&mut s, KeyCode::BackTab);
        type_text(&mut s, "APP-14");
        press(&mut s, KeyCode::Enter);
        assert_eq!(s.modal, Modal::None);
        assert_eq!(s.tasks.len(), 4);
        assert_eq!(s.active, 3);
        assert_eq!(s.focus, Pane::Agent);
        let t = &s.tasks[3];
        assert_eq!(t.name, "APP-14");
        assert_eq!(t.agent, "codex");
        assert_eq!(t.status, AgentStatus::Idle);
        assert_eq!(t.chat.len(), 1);
        assert_eq!(s.toasts.last().map(|t| t.level), Some(ToastLevel::Info));
    }

    #[test]
    fn new_task_esc_closes() {
        let mut s = demo_state();
        update(&mut s, Action::NewTask);
        press(&mut s, KeyCode::Esc);
        assert_eq!(s.modal, Modal::None);
    }

    #[test]
    fn blank_input_sends_nothing() {
        let mut s = demo_state();
        s.active = 2;
        s.focus = Pane::Agent;
        let before = s.tasks[2].chat.len();
        press(&mut s, KeyCode::Enter);
        type_text(&mut s, "   ");
        press(&mut s, KeyCode::Enter);
        assert_eq!(s.tasks[2].chat.len(), before);
    }

    #[test]
    fn input_is_sent_with_simulated_reply() {
        let mut s = demo_state();
        s.active = 2;
        s.focus = Pane::Agent;
        let before = s.tasks[2].chat.len();
        type_text(&mut s, "hola");
        update(
            &mut s,
            Action::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
        );
        type_text(&mut s, "x");
        assert_eq!(s.tasks[2].input, "hola\nx");
        press(&mut s, KeyCode::Enter);
        assert!(s.tasks[2].input.is_empty());
        assert_eq!(s.tasks[2].chat.len(), before + 2);
        assert_eq!(
            s.tasks[2].chat[before],
            ChatItem::User("hola\nx".to_string())
        );
    }

    #[test]
    fn permission_is_resolved_with_selected_option() {
        let mut s = demo_state();
        s.active = 1;
        s.focus = Pane::Agent;
        press(&mut s, KeyCode::Down);
        press(&mut s, KeyCode::Enter);
        let task = &s.tasks[1];
        assert_eq!(task.status, AgentStatus::Working);
        assert!(task.chat.iter().any(|c| matches!(
            c,
            ChatItem::Permission {
                resolved: Some(1),
                ..
            }
        )));
        assert_eq!(
            s.toasts.last().map(|t| t.text.as_str()),
            Some("Permiso respondido: Permitir siempre")
        );
    }

    #[test]
    fn esc_cancels_working_turn() {
        let mut s = demo_state();
        s.focus = Pane::Agent;
        press(&mut s, KeyCode::Esc);
        assert_eq!(s.tasks[0].status, AgentStatus::Idle);
        assert_eq!(
            s.tasks[0].chat.last(),
            Some(&ChatItem::Notice("Turno cancelado".to_string()))
        );
    }

    #[test]
    fn demo_status_needs_permission_adds_prompt() {
        let mut s = demo_state();
        update(&mut s, Action::DemoSetStatus(AgentStatus::NeedsPermission));
        assert_eq!(s.tasks[0].status, AgentStatus::NeedsPermission);
        assert!(matches!(
            s.tasks[0].chat.last(),
            Some(ChatItem::Permission { resolved: None, .. })
        ));
    }

    #[test]
    fn review_discard_with_confirm_removes_task() {
        let mut s = demo_state();
        update(&mut s, Action::OpenReview);
        assert!(matches!(s.modal, Modal::Review { .. }));
        press(&mut s, KeyCode::Char('x'));
        assert!(matches!(s.modal, Modal::Confirm { .. }));
        press(&mut s, KeyCode::Char('s'));
        assert_eq!(s.modal, Modal::None);
        assert_eq!(s.tasks.len(), 2);
        assert_eq!(s.tasks[0].name, "APP-13");
    }

    #[test]
    fn review_discard_cancel_keeps_task() {
        let mut s = demo_state();
        update(&mut s, Action::OpenReview);
        press(&mut s, KeyCode::Char('x'));
        press(&mut s, KeyCode::Char('n'));
        assert_eq!(s.tasks.len(), 3);
        assert_eq!(s.modal, Modal::None);
    }

    #[test]
    fn discarding_last_task_opens_new_task() {
        let mut s = demo_state();
        s.tasks.truncate(1);
        update(&mut s, Action::OpenReview);
        press(&mut s, KeyCode::Char('x'));
        press(&mut s, KeyCode::Enter);
        assert!(s.tasks.is_empty());
        assert!(matches!(s.modal, Modal::NewTask { .. }));
    }

    #[test]
    fn review_actions_show_demo_toast_and_navigate() {
        let mut s = demo_state();
        update(&mut s, Action::OpenReview);
        press(&mut s, KeyCode::Char('j'));
        assert!(matches!(s.modal, Modal::Review { selected: 1, .. }));
        press(&mut s, KeyCode::Char('k'));
        press(&mut s, KeyCode::Char('c'));
        assert_eq!(
            s.toasts.last().map(|t| t.text.as_str()),
            Some("DEMO: no se ejecutó commit")
        );
        press(&mut s, KeyCode::Esc);
        assert_eq!(s.modal, Modal::None);
    }

    #[test]
    fn toasts_expire_on_tick_and_can_be_shown_again() {
        let mut s = demo_state();
        s.toasts.push(Toast {
            text: "hola".to_string(),
            level: ToastLevel::Info,
            ttl_ticks: 2,
        });
        s.last_toast = s.toasts.last().cloned();
        update(&mut s, Action::Tick);
        assert_eq!(s.toasts.len(), 1);
        update(&mut s, Action::Tick);
        assert!(s.toasts.is_empty());
        assert!(s.last_toast.is_some());
        update(&mut s, Action::ShowLastToast);
        assert_eq!(s.toasts.len(), 1);
        assert_eq!(s.toasts[0].ttl_ticks, TOAST_TTL_TICKS);
    }

    #[test]
    fn quit_sets_flag_and_effect() {
        let mut s = demo_state();
        assert_eq!(update(&mut s, Action::Quit), vec![Effect::Quit]);
        assert!(s.should_quit);
    }

    #[test]
    fn help_toggles_and_blocks_navigation() {
        let mut s = demo_state();
        update(&mut s, Action::Help);
        assert_eq!(s.modal, Modal::Help);
        update(&mut s, Action::SelectTask(1));
        assert_eq!(s.active, 0);
        update(&mut s, Action::Help);
        assert_eq!(s.modal, Modal::None);
    }
}
