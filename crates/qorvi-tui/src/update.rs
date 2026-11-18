//! `update`: transforma el estado según una acción. No hace IO: lo que necesita del
//! mundo (agentes, paneles, git) lo pide como [`Effect`] y el runtime lo ejecuta.
//!
//! En la demo (`state.demo`) todo se simula y no se emiten efectos de IO.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use qorvi_acp::{AgentEvent, StopReason};
use qorvi_core::{SessionCost, SessionInfo};
use ratatui::layout::{Position, Rect};

use crate::action::{Action, PaneInput};
use crate::demo::fake_changes;
use crate::explorer::{visible_rows, RowKind};
use crate::layout::{self, Areas};
use crate::state::{
    AgentMode, AgentStatus, AppState, ChatItem, ConfirmAction, Modal, Pane, PermissionRef, Task,
    ToastLevel, ToolStatus,
};
use crate::view::{chat, explorer as explorer_view, modals, review, Tier};

/// Líneas que desplaza una muesca de la rueda en el historial del chat.
const WHEEL_LINES: usize = 3;

/// Peticiones de IO para el runtime. Las tareas se nombran por su nombre (estable aunque
/// cambie su posición en la lista).
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Quit,
    /// La tarea pasó a ser la activa: arrancar su agente y sus paneles si no corren.
    Activate(String),
    Prompt(String, String),
    Cancel(String),
    /// Respuesta a un permiso: id y opción (`None` = descartar).
    Permission(String, u64, Option<String>),
    SetMode(String, String),
    CreateTask {
        name: String,
        agent: String,
        base: String,
    },
    RemoveTask(String),
    OpenFile(String, String),
    RefreshFiles(String),
    LoadReview(String),
    Commit(String),
    Merge(String),
    PullRequest(String),
    RecordSession(String, SessionInfo),
    PaneInput(String, Pane, PaneInput),
    RestartPane(String, Pane),
}

pub fn update(state: &mut AppState, action: Action) -> Vec<Effect> {
    let mut fx = Vec::new();
    let modal_open = state.modal != Modal::None;
    // Cualquier acción resuelta consume el prefijo; el tick, el resize y los eventos
    // del runtime no.
    if matches!(
        action,
        Action::Key(_)
            | Action::Mouse(_)
            | Action::Paste(_)
            | Action::PrefixCancel
            | Action::PrefixLiteral
            | Action::PrefixUnknown(_)
            | Action::FocusLeft
            | Action::FocusRight
            | Action::FocusUp
            | Action::FocusDown
            | Action::SelectTask(_)
            | Action::NewTask
            | Action::OpenReview
            | Action::ToggleZoom
            | Action::ToggleTerminal
            | Action::ToggleExplorer
            | Action::ShowLastToast
            | Action::Quit
            | Action::Help
            | Action::DemoSetStatus(_)
    ) {
        state.prefix_pending = false;
    }
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
        Action::PrefixStart => state.prefix_pending = true,
        Action::PrefixCancel => {}
        Action::PrefixLiteral => {
            if state.demo {
                state.push_toast("DEMO: se enviaría Ctrl+Space al panel", ToastLevel::Info);
            } else if !modal_open {
                pane_input(state, &mut fx, PaneInput::Bytes(vec![0]));
            }
        }
        Action::PrefixUnknown(name) => {
            state.push_toast(
                format!("Atajo desconocido: Ctrl+Space {name}"),
                ToastLevel::Warning,
            );
        }
        Action::DemoSetStatus(status) => {
            if state.demo {
                set_status(state, status)
            }
        }
        Action::Key(key) => {
            alt_hint(state, &key);
            handle_key(state, key, &mut fx)
        }
        Action::Mouse(mouse) => handle_mouse(state, mouse, &mut fx),
        Action::Paste(text) => paste(state, text, &mut fx),
        Action::Agent(name, ev) => agent_event(state, &name, ev, &mut fx),
        Action::AgentStarting(name) => {
            if let Some(t) = task_mut(state, &name) {
                t.status = AgentStatus::Connecting;
            }
        }
        Action::AgentFailed(name, msg) => {
            if let Some(t) = task_mut(state, &name) {
                t.status = AgentStatus::Error;
                t.chat.push(ChatItem::Notice(msg.clone()));
            }
            state.push_toast(msg, ToastLevel::Error);
        }
        Action::TaskCreated(task, warnings) => {
            let name = task.name.clone();
            state.tasks.push(*task);
            state.active = state.tasks.len() - 1;
            state.focus = Pane::Agent;
            state.push_toast(format!("Tarea {name} creada"), ToastLevel::Info);
            for w in warnings {
                state.push_toast(w, ToastLevel::Warning);
            }
            fx.push(Effect::Activate(name));
        }
        Action::TaskRemoved(name) => {
            if let Some(idx) = state.tasks.iter().position(|t| t.name == name) {
                remove_task_at(state, idx, &mut fx);
                state.push_toast(format!("Tarea {name} descartada"), ToastLevel::Info);
            }
        }
        Action::FilesLoaded(name, files) => {
            if let Some(t) = task_mut(state, &name) {
                t.files = files;
            }
        }
        Action::ReviewLoaded(name, files) => {
            let active = state.active_task().is_some_and(|t| t.name == name);
            if active && matches!(state.modal, Modal::None | Modal::Review { .. }) {
                let selected = match &state.modal {
                    Modal::Review { selected, .. } => {
                        (*selected).min(files.len().saturating_sub(1))
                    }
                    _ => 0,
                };
                state.modal = Modal::Review { files, selected };
            }
        }
        Action::PaneNote(name, pane, note) => {
            if let Some(t) = task_mut(state, &name) {
                match pane {
                    Pane::Editor => t.editor_note = note,
                    Pane::Terminal => t.terminal_note = note,
                    _ => {}
                }
            }
        }
        Action::EditorOpened(name, path) => {
            if let Some(t) = task_mut(state, &name) {
                t.editor_file = Some(path);
            }
        }
        Action::Toast(text, level) => state.push_toast(text, level),
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
        Action::SelectTask(i) => select_task(state, i, &mut fx),
        Action::NewTask => open_new_task(state),
        Action::OpenReview => {
            if let Some(task) = state.active_task() {
                if state.demo {
                    state.modal = Modal::Review {
                        files: fake_changes(),
                        selected: 0,
                    };
                } else {
                    fx.push(Effect::LoadReview(task.name.clone()));
                }
            }
        }
        Action::ToggleZoom => state.zoom = !state.zoom,
        Action::ToggleTerminal => state.terminal_minimized = !state.terminal_minimized,
        Action::ToggleExplorer => toggle_explorer(state),
    }
    fx
}

fn task_mut<'a>(state: &'a mut AppState, name: &str) -> Option<&'a mut Task> {
    state.tasks.iter_mut().find(|t| t.name == name)
}

fn active_name(state: &AppState) -> Option<String> {
    state.active_task().map(|t| t.name.clone())
}

fn tick(state: &mut AppState) {
    for t in &mut state.toasts {
        t.ttl_ticks = t.ttl_ticks.saturating_sub(1);
    }
    state.toasts.retain(|t| t.ttl_ticks > 0);
}

fn toggle_explorer(state: &mut AppState) {
    if state.compact {
        // En compacto el explorador es una pestaña: Alt+e entra y sale de ella.
        state.focus = if state.focus == Pane::Explorer {
            Pane::Editor
        } else {
            Pane::Explorer
        };
        return;
    }
    state.explorer_toggled = !state.explorer_toggled;
    if !state.explorer_visible() && state.focus == Pane::Explorer {
        state.focus = Pane::Editor;
    }
}

fn focus_horizontal(state: &mut AppState, right: bool) {
    const ORDER: [Pane; 4] = [Pane::Explorer, Pane::Editor, Pane::Agent, Pane::Terminal];
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
        (Pane::Editor, false) if state.explorer_visible() => Pane::Explorer,
        (Pane::Explorer, true) => Pane::Editor,
        (Pane::Editor, true) => Pane::Agent,
        (Pane::Agent, false) => Pane::Editor,
        (other, _) => other,
    };
}

fn select_task(state: &mut AppState, i: usize, fx: &mut Vec<Effect>) {
    if i >= state.tasks.len() {
        return;
    }
    state.active = i;
    if state.tasks[i].status == AgentStatus::Done {
        state.tasks[i].status = AgentStatus::Idle;
    }
    if !state.demo {
        fx.push(Effect::Activate(state.tasks[i].name.clone()));
    }
}

fn open_new_task(state: &mut AppState) {
    state.modal = Modal::NewTask {
        name: String::new(),
        agent_idx: 0,
        base: state.default_base.clone(),
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
                request: None,
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

/// Lo que escribe Option+tecla en macOS (distribución US) con las letras de los atajos,
/// cuando la terminal no trata Option como Alt/Meta. Solo símbolos que casi nadie
/// escribe a propósito (sin `¡`, `¿` ni `ª`, que son texto normal en español).
const OPTION_CHARS: &str = "˙∆˚¬∂Ωœ†∫µ™∞¶®";

/// Avisa una vez si llega un carácter que delata un Alt mal configurado. La tecla sigue
/// su curso (puede ser texto de verdad).
fn alt_hint(state: &mut AppState, key: &KeyEvent) {
    let KeyCode::Char(c) = key.code else {
        return;
    };
    if state.alt_warned || !is_text_modifier(key.modifiers) || !OPTION_CHARS.contains(c) {
        return;
    }
    state.alt_warned = true;
    state.push_toast(
        format!(
            "Tu terminal escribe «{c}» con Alt/Option: usa Ctrl+Space + tecla o configura \
             Option como Meta (qorvi doctor keys)"
        ),
        ToastLevel::Warning,
    );
}

fn is_text_modifier(m: KeyModifiers) -> bool {
    (m - KeyModifiers::SHIFT).is_empty()
}

/// Envía `input` al panel con foco (editor o terminal) de la tarea activa.
fn pane_input(state: &AppState, fx: &mut Vec<Effect>, input: PaneInput) {
    if let (Some(name), pane @ (Pane::Editor | Pane::Terminal)) = (active_name(state), state.focus)
    {
        fx.push(Effect::PaneInput(name, pane, input));
    }
}

fn handle_key(state: &mut AppState, key: KeyEvent, fx: &mut Vec<Effect>) {
    match state.modal {
        Modal::None => match state.focus {
            Pane::Agent => agent_key(state, key, fx),
            Pane::Explorer => explorer_key(state, key, fx),
            Pane::Editor | Pane::Terminal => {
                // En la demo las teclas se ignoran.
                if state.demo {
                    return;
                }
                let Some(task) = state.active_task() else {
                    return;
                };
                let note = match state.focus {
                    Pane::Editor => &task.editor_note,
                    _ => &task.terminal_note,
                };
                if note.is_some() {
                    if key.code == KeyCode::Enter {
                        fx.push(Effect::RestartPane(task.name.clone(), state.focus));
                    }
                } else {
                    pane_input(state, fx, PaneInput::Key(key));
                }
            }
        },
        Modal::NewTask { .. } => new_task_key(state, key, fx),
        Modal::Confirm { .. } => confirm_key(state, key, fx),
        Modal::Review { .. } => review_key(state, key, fx),
        Modal::Help => {
            if key.code == KeyCode::Esc {
                state.modal = Modal::None;
            }
        }
    }
}

fn paste(state: &mut AppState, text: String, fx: &mut Vec<Effect>) {
    match &mut state.modal {
        Modal::NewTask {
            name, base, field, ..
        } => {
            let line: String = text.lines().next().unwrap_or_default().trim().to_string();
            match *field {
                0 => name.push_str(&line),
                2 => base.push_str(&line),
                _ => {}
            }
        }
        Modal::None => match state.focus {
            Pane::Agent => {
                if let Some(t) = state.active_task_mut() {
                    t.input.push_str(&text.replace("\r\n", "\n"));
                    t.chat_scroll = 0;
                }
            }
            Pane::Editor | Pane::Terminal if !state.demo => {
                pane_input(state, fx, PaneInput::Paste(text));
            }
            _ => {}
        },
        _ => {}
    }
}

fn explorer_key(state: &mut AppState, key: KeyEvent, fx: &mut Vec<Effect>) {
    if !is_text_modifier(key.modifiers) {
        return;
    }
    let demo = state.demo;
    let mut toast = None;
    let mut open_in_editor = false;
    {
        let Some(task) = state.active_task_mut() else {
            return;
        };
        if key.code == KeyCode::Char('r') {
            if demo {
                toast = Some("DEMO: árbol refrescado".to_string());
            } else {
                fx.push(Effect::RefreshFiles(task.name.clone()));
            }
        }
        let rows = visible_rows(&task.files, &task.explorer.expanded);
        if rows.is_empty() {
            if let Some(text) = toast {
                state.push_toast(text, ToastLevel::Info);
            }
            return;
        }
        let ex = &mut task.explorer;
        ex.selected = ex.selected.min(rows.len() - 1);
        let row = &rows[ex.selected];
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                ex.selected = (ex.selected + 1).min(rows.len() - 1)
            }
            KeyCode::Char('k') | KeyCode::Up => ex.selected = ex.selected.saturating_sub(1),
            KeyCode::Char('l') | KeyCode::Enter if row.is_dir() => {
                if !ex.expanded.remove(&row.path) {
                    ex.expanded.insert(row.path.clone());
                }
            }
            KeyCode::Enter => {
                if demo {
                    task.editor_file = Some(row.path.clone());
                    toast = Some(format!("DEMO: se abriría {} en nvim (--remote)", row.path));
                } else {
                    fx.push(Effect::OpenFile(task.name.clone(), row.path.clone()));
                }
                open_in_editor = true;
            }
            KeyCode::Char('h') => {
                if matches!(row.kind, RowKind::Dir { expanded: true, .. }) {
                    ex.expanded.remove(&row.path);
                } else if let Some(parent) = row.parent_path() {
                    if let Some(i) = rows.iter().position(|r| r.path == parent) {
                        ex.selected = i;
                    }
                }
            }
            _ => {}
        }
    }
    if open_in_editor {
        state.focus = Pane::Editor;
    }
    if let Some(text) = toast {
        state.push_toast(text, ToastLevel::Info);
    }
}

fn last_unresolved_permission(task: &Task) -> Option<usize> {
    task.chat
        .iter()
        .rposition(|c| matches!(c, ChatItem::Permission { resolved: None, .. }))
}

fn agent_key(state: &mut AppState, key: KeyEvent, fx: &mut Vec<Effect>) {
    let demo = state.demo;
    let Some(task) = state.active_task_mut() else {
        return;
    };
    let perm_idx = last_unresolved_permission(task);

    match key.code {
        KeyCode::Esc => {
            if demo {
                if task.status == AgentStatus::Working {
                    task.status = AgentStatus::Idle;
                    task.chat
                        .push(ChatItem::Notice("Turno cancelado".to_string()));
                }
            } else if matches!(
                task.status,
                AgentStatus::Working | AgentStatus::NeedsPermission
            ) {
                fx.push(Effect::Cancel(task.name.clone()));
            }
        }
        KeyCode::BackTab if !demo => {
            // Shift+Tab: siguiente modo de permisos del agente.
            if task.modes.is_empty() {
                state.push_toast("El agente no tiene modos de permisos", ToastLevel::Info);
                return;
            }
            let cur = task
                .mode
                .as_ref()
                .and_then(|m| task.modes.iter().position(|x| &x.id == m));
            let next = cur.map_or(0, |i| (i + 1) % task.modes.len());
            fx.push(Effect::SetMode(
                task.name.clone(),
                task.modes[next].id.clone(),
            ));
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
                    request,
                    ..
                }) = task.chat.get_mut(i)
                {
                    *resolved = Some(*selected);
                    answered = options.get(*selected).cloned();
                    if let Some(r) = request {
                        fx.push(Effect::Permission(
                            task.name.clone(),
                            r.id,
                            r.option_ids.get(*selected).cloned(),
                        ));
                    }
                }
                task.status = AgentStatus::Working;
                if let Some(option) = answered {
                    state.push_toast(format!("Permiso respondido: {option}"), ToastLevel::Info);
                }
            } else if !task.input.trim().is_empty() {
                if demo {
                    let text = std::mem::take(&mut task.input);
                    task.chat_scroll = 0;
                    task.chat.push(ChatItem::User(text));
                    task.chat
                        .push(ChatItem::Agent("DEMO: respuesta simulada.".to_string()));
                } else if task.status == AgentStatus::Working {
                    state.push_toast(
                        "El agente está trabajando: espera o cancela con Esc",
                        ToastLevel::Warning,
                    );
                } else {
                    let text = std::mem::take(&mut task.input);
                    task.chat_scroll = 0;
                    task.chat.push(ChatItem::User(text.clone()));
                    task.status = AgentStatus::Working;
                    fx.push(Effect::Prompt(task.name.clone(), text));
                }
            }
        }
        KeyCode::Backspace => {
            task.input.pop();
            task.chat_scroll = 0;
        }
        KeyCode::Char(c) if is_text_modifier(key.modifiers) => {
            task.input.push(c);
            task.chat_scroll = 0;
        }
        _ => {}
    }
}

/// Añade texto al último mensaje si es del mismo tipo; si no, abre uno nuevo.
fn append_chunk(task: &mut Task, text: String, user: bool) {
    match task.chat.last_mut() {
        Some(ChatItem::User(t)) if user => t.push_str(&text),
        Some(ChatItem::Agent(t)) if !user => t.push_str(&text),
        _ => task.chat.push(if user {
            ChatItem::User(text)
        } else {
            ChatItem::Agent(text)
        }),
    }
}

fn tool_status(s: qorvi_acp::ToolStatus) -> ToolStatus {
    match s {
        qorvi_acp::ToolStatus::Pending => ToolStatus::Pending,
        qorvi_acp::ToolStatus::InProgress => ToolStatus::InProgress,
        qorvi_acp::ToolStatus::Completed => ToolStatus::Completed,
        qorvi_acp::ToolStatus::Failed => ToolStatus::Failed,
    }
}

/// Descarta los permisos sin responder (`id` = solo ese; `None` = todos).
fn discard_permissions(task: &mut Task, id: Option<u64>) {
    for item in &mut task.chat {
        if let ChatItem::Permission {
            options,
            resolved: resolved @ None,
            request,
            ..
        } = item
        {
            if id.is_none() || request.as_ref().map(|r| r.id) == id {
                *resolved = Some(options.len());
            }
        }
    }
}

fn agent_event(state: &mut AppState, name: &str, ev: AgentEvent, fx: &mut Vec<Effect>) {
    let is_active = state.active_task().is_some_and(|t| t.name == name);
    let Some(task) = task_mut(state, name) else {
        return;
    };
    let mut toast: Option<(String, ToastLevel)> = None;
    let session = |info: SessionInfo| Effect::RecordSession(name.to_string(), info);
    match ev {
        AgentEvent::Ready(r) => {
            if matches!(
                task.status,
                AgentStatus::Connecting | AgentStatus::Disconnected | AgentStatus::Error
            ) {
                task.status = AgentStatus::Idle;
            }
            if let Some(m) = r.modes {
                task.mode = Some(m.current);
                task.modes = m
                    .available
                    .into_iter()
                    .map(|m| AgentMode {
                        id: m.id,
                        name: m.name,
                    })
                    .collect();
            }
            if r.resumed {
                task.chat
                    .push(ChatItem::Notice("Sesión reanudada".to_string()));
            }
            fx.push(session(SessionInfo {
                session_id: Some(r.session_id),
                ..Default::default()
            }));
        }
        AgentEvent::UserChunk(t) => append_chunk(task, t, true),
        AgentEvent::MessageChunk(t) => append_chunk(task, t, false),
        AgentEvent::ThoughtChunk(_) | AgentEvent::Plan(_) | AgentEvent::Stderr(_) => {}
        AgentEvent::ToolCall(tc) => task.chat.push(ChatItem::ToolCall {
            id: tc.id,
            title: tc.title,
            kind: tc.kind,
            status: tool_status(tc.status),
            expanded: false,
        }),
        AgentEvent::ToolCallUpdate(p) => {
            let found = task
                .chat
                .iter_mut()
                .rev()
                .find(|c| matches!(c, ChatItem::ToolCall { id, .. } if *id == p.id));
            if let Some(ChatItem::ToolCall {
                title,
                kind,
                status,
                ..
            }) = found
            {
                if let Some(t) = p.title {
                    *title = t;
                }
                if let Some(k) = p.kind {
                    *kind = k;
                }
                if let Some(s) = p.status {
                    *status = tool_status(s);
                }
            }
        }
        AgentEvent::PermissionRequest(req) => {
            task.chat.push(ChatItem::Permission {
                title: req.title,
                options: req.options.iter().map(|o| o.name.clone()).collect(),
                selected: 0,
                resolved: None,
                request: Some(PermissionRef {
                    id: req.id,
                    option_ids: req.options.into_iter().map(|o| o.id).collect(),
                }),
            });
            task.status = AgentStatus::NeedsPermission;
            if !is_active {
                toast = Some((
                    format!("Tarea {name} esperando permiso"),
                    ToastLevel::Warning,
                ));
            }
        }
        AgentEvent::PermissionCancelled(id) => discard_permissions(task, Some(id)),
        AgentEvent::TurnEnded(reason) => {
            discard_permissions(task, None);
            let note = match reason {
                StopReason::EndTurn => None,
                StopReason::Cancelled => Some("Turno cancelado"),
                StopReason::Refusal => Some("El agente rechazó continuar"),
                StopReason::MaxTokens => Some("El agente llegó al límite de tokens"),
                StopReason::MaxTurnRequests => Some("El agente llegó al límite de peticiones"),
                StopReason::Other => Some("El turno terminó"),
            };
            if let Some(n) = note {
                task.chat.push(ChatItem::Notice(n.to_string()));
            }
            task.status = if is_active {
                AgentStatus::Idle
            } else {
                toast = Some((format!("Tarea {name} terminó"), ToastLevel::Info));
                AgentStatus::Done
            };
            fx.push(Effect::RefreshFiles(name.to_string()));
        }
        AgentEvent::TurnFailed(msg) => {
            discard_permissions(task, None);
            task.status = AgentStatus::Error;
            task.chat
                .push(ChatItem::Notice(format!("El turno falló: {msg}")));
            toast = Some((format!("El agente de {name} falló"), ToastLevel::Error));
        }
        AgentEvent::ModeChanged(m) => {
            let label = task
                .modes
                .iter()
                .find(|x| x.id == m)
                .map_or(m.clone(), |x| x.name.clone());
            task.chat
                .push(ChatItem::Notice(format!("Modo de permisos: {label}")));
            task.mode = Some(m);
        }
        AgentEvent::Usage(u) => {
            if let Some(c) = u.cost {
                task.cost_usd = c.amount;
                fx.push(session(SessionInfo {
                    cost: Some(SessionCost {
                        amount: c.amount,
                        currency: c.currency,
                    }),
                    ..Default::default()
                }));
            }
        }
        AgentEvent::Title(t) => {
            task.title = t.clone();
            fx.push(session(SessionInfo {
                title: Some(t),
                ..Default::default()
            }));
        }
        AgentEvent::FsDenied { path, reason } => task.chat.push(ChatItem::Notice(format!(
            "Acceso denegado a {}: {reason}",
            path.display()
        ))),
        AgentEvent::Notice(n) => task.chat.push(ChatItem::Notice(n)),
        AgentEvent::Exited { error, .. } => {
            discard_permissions(task, None);
            match error {
                Some(e) => {
                    task.status = AgentStatus::Error;
                    task.chat
                        .push(ChatItem::Notice(format!("El agente se cerró: {e}")));
                    toast = Some((format!("El agente de {name} se cerró"), ToastLevel::Error));
                }
                None => task.status = AgentStatus::Disconnected,
            }
        }
    }
    if let Some((text, level)) = toast {
        state.push_toast(text, level);
    }
}

/// Rectángulo interior del editor o la terminal bajo `pos`, si lo hay.
fn pane_inner_at(areas: &Areas, pos: Position) -> Option<(Pane, Rect)> {
    let (pane, rect) = areas.pane_at(pos.x, pos.y)?;
    if !matches!(pane, Pane::Editor | Pane::Terminal)
        || (pane == Pane::Terminal && areas.terminal_minimized)
    {
        return None;
    }
    let inner = layout::inner(rect);
    inner.contains(pos).then_some((pane, inner))
}

fn relative(mouse: MouseEvent, inner: Rect) -> MouseEvent {
    MouseEvent {
        column: mouse.column - inner.x,
        row: mouse.row - inner.y,
        ..mouse
    }
}

fn handle_mouse(state: &mut AppState, mouse: MouseEvent, fx: &mut Vec<Effect>) {
    let areas = layout::compute(Rect::new(0, 0, state.size.0, state.size.1), state);
    if areas.tier == Tier::TooSmall {
        return;
    }
    let pos = Position::new(mouse.column, mouse.row);
    match state.modal {
        Modal::None => match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => click(state, &areas, pos, mouse, fx),
            MouseEventKind::ScrollUp => wheel(state, &areas, pos, true, mouse, fx),
            MouseEventKind::ScrollDown => wheel(state, &areas, pos, false, mouse, fx),
            // Arrastres, otros botones y soltar: solo le interesan al programa del panel.
            _ => {
                if let Some((pane, inner)) = pane_inner_at(&areas, pos) {
                    if !state.demo && pane == state.focus {
                        pane_input(state, fx, PaneInput::Mouse(relative(mouse, inner)));
                    }
                }
            }
        },
        // Un clic fuera del diálogo lo cierra; NewTask y Confirm no, para no perder lo escrito.
        Modal::Help => {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && !modals::help_rect(areas.body).contains(pos)
            {
                state.modal = Modal::None;
            }
        }
        Modal::Review { .. } => review_mouse(state, &areas, mouse.kind, pos),
        Modal::NewTask { .. } | Modal::Confirm { .. } => {}
    }
}

fn click(
    state: &mut AppState,
    areas: &Areas,
    pos: Position,
    mouse: MouseEvent,
    fx: &mut Vec<Effect>,
) {
    if let Some(tab) = areas.task_tabs.iter().find(|t| t.rect.contains(pos)) {
        select_task(state, tab.index, fx);
        return;
    }
    if let Some((_, pane)) = areas.compact_tabs.iter().find(|(r, _)| r.contains(pos)) {
        state.focus = *pane;
        return;
    }
    let Some((pane, rect)) = areas.pane_at(pos.x, pos.y) else {
        return;
    };
    state.focus = pane;
    match pane {
        Pane::Explorer => click_explorer(state, rect, pos, fx),
        Pane::Agent => click_agent(state, rect, pos, fx),
        Pane::Editor | Pane::Terminal => {
            if let Some((_, inner)) = pane_inner_at(areas, pos) {
                if !state.demo {
                    pane_input(state, fx, PaneInput::Mouse(relative(mouse, inner)));
                }
            }
        }
    }
}

fn click_explorer(state: &mut AppState, rect: Rect, pos: Position, fx: &mut Vec<Effect>) {
    let inner = layout::inner(rect);
    if !inner.contains(pos) {
        return;
    }
    let Some(task) = state.active_task_mut() else {
        return;
    };
    let rows = visible_rows(&task.files, &task.explorer.expanded);
    if rows.is_empty() {
        return;
    }
    let selected = task.explorer.selected.min(rows.len() - 1);
    let start = explorer_view::scroll_start(selected, usize::from(inner.height));
    let idx = start + usize::from(pos.y - inner.y);
    if idx >= rows.len() {
        return;
    }
    task.explorer.selected = idx;
    if idx == selected {
        // Segundo clic sobre la fila seleccionada: como Enter.
        explorer_key(state, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), fx);
    }
}

fn click_agent(state: &mut AppState, rect: Rect, pos: Position, fx: &mut Vec<Effect>) {
    let Some(task) = state.active_task() else {
        return;
    };
    let view = chat::chat_view(task, layout::inner(rect), state.ascii);
    let Some(option) = view
        .options
        .iter()
        .find(|(r, _)| r.contains(pos))
        .map(|(_, i)| *i)
    else {
        return;
    };
    let Some(idx) = last_unresolved_permission(task) else {
        return;
    };
    let Some(task) = state.active_task_mut() else {
        return;
    };
    let Some(ChatItem::Permission { selected, .. }) = task.chat.get_mut(idx) else {
        return;
    };
    if *selected == option {
        agent_key(state, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), fx);
    } else {
        *selected = option;
    }
}

fn wheel(
    state: &mut AppState,
    areas: &Areas,
    pos: Position,
    up: bool,
    mouse: MouseEvent,
    fx: &mut Vec<Effect>,
) {
    let Some((pane, rect)) = areas.pane_at(pos.x, pos.y) else {
        return;
    };
    if matches!(pane, Pane::Editor | Pane::Terminal) {
        // La rueda llega al programa (o al historial) del panel bajo el ratón.
        if let (false, Some((pane, inner)), Some(name)) =
            (state.demo, pane_inner_at(areas, pos), active_name(state))
        {
            fx.push(Effect::PaneInput(
                name,
                pane,
                PaneInput::Wheel(relative(mouse, inner)),
            ));
        }
        return;
    }
    let ascii = state.ascii;
    let Some(task) = state.active_task_mut() else {
        return;
    };
    match pane {
        Pane::Explorer => {
            let n = visible_rows(&task.files, &task.explorer.expanded).len();
            if n > 0 {
                let ex = &mut task.explorer;
                let cur = ex.selected.min(n - 1);
                ex.selected = if up {
                    cur.saturating_sub(1)
                } else {
                    (cur + 1).min(n - 1)
                };
            }
        }
        Pane::Agent => {
            let max = chat::chat_view(task, layout::inner(rect), ascii).max_scroll;
            let cur = task.chat_scroll.min(max);
            task.chat_scroll = if up {
                (cur + WHEEL_LINES).min(max)
            } else {
                cur.saturating_sub(WHEEL_LINES)
            };
        }
        Pane::Editor | Pane::Terminal => {}
    }
}

fn review_mouse(state: &mut AppState, areas: &Areas, kind: MouseEventKind, pos: Position) {
    let zones = review::areas(areas.body);
    let Modal::Review { files, selected } = &mut state.modal else {
        return;
    };
    match kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if !areas.body.contains(pos) {
                state.modal = Modal::None;
                return;
            }
            let list = layout::inner(zones.list);
            if list.contains(pos) {
                let idx = usize::from(pos.y - list.y);
                if idx < files.len() {
                    *selected = idx;
                }
            }
        }
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown if zones.list.contains(pos) => {
            if kind == MouseEventKind::ScrollUp {
                *selected = selected.saturating_sub(1);
            } else if *selected + 1 < files.len() {
                *selected += 1;
            }
        }
        _ => {}
    }
}

fn new_task_key(state: &mut AppState, key: KeyEvent, fx: &mut Vec<Effect>) {
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
            create_task(state, &name, idx, &base, fx);
        }
        _ => {}
    }
}

fn create_task(
    state: &mut AppState,
    name: &str,
    agent_idx: usize,
    base: &str,
    fx: &mut Vec<Effect>,
) {
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
    if !state.demo {
        // El runtime crea el worktree y responde con `TaskCreated` o un aviso de error
        // (el diálogo se cierra igualmente: el error dice qué corregir).
        state.modal = Modal::None;
        fx.push(Effect::CreateTask {
            name: name.to_string(),
            agent,
            base: base.trim().to_string(),
        });
        return;
    }
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

fn review_key(state: &mut AppState, key: KeyEvent, fx: &mut Vec<Effect>) {
    let active = state.active;
    let task_name = active_name(state);
    let demo = state.demo;
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
    if demo {
        state.push_toast(
            format!("DEMO: no se ejecutó {action_name}"),
            ToastLevel::Info,
        );
        return;
    }
    let Some(name) = task_name else {
        return;
    };
    fx.push(match key.code {
        KeyCode::Char('c') => Effect::Commit(name),
        KeyCode::Char('m') => Effect::Merge(name),
        _ => Effect::PullRequest(name),
    });
}

fn confirm_key(state: &mut AppState, key: KeyEvent, fx: &mut Vec<Effect>) {
    let Modal::Confirm { action, .. } = state.modal else {
        return;
    };
    match key.code {
        KeyCode::Char('s') | KeyCode::Char('S') | KeyCode::Enter => {
            state.modal = Modal::None;
            match action {
                ConfirmAction::DiscardTask(idx) => discard_task(state, idx, fx),
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => state.modal = Modal::None,
        _ => {}
    }
}

fn discard_task(state: &mut AppState, idx: usize, fx: &mut Vec<Effect>) {
    if idx >= state.tasks.len() {
        return;
    }
    if !state.demo {
        // El runtime borra worktree y rama, y responde con `TaskRemoved`.
        fx.push(Effect::RemoveTask(state.tasks[idx].name.clone()));
        return;
    }
    let name = state.tasks[idx].name.clone();
    remove_task_at(state, idx, fx);
    state.push_toast(format!("Tarea {name} descartada (DEMO)"), ToastLevel::Info);
}

/// Quita la tarea `idx` de la lista y reajusta la tarea activa.
fn remove_task_at(state: &mut AppState, idx: usize, fx: &mut Vec<Effect>) {
    let was_active = idx == state.active;
    state.tasks.remove(idx);
    if state.tasks.is_empty() {
        state.active = 0;
        open_new_task(state);
        return;
    }
    if idx < state.active {
        state.active -= 1;
    }
    state.active = state.active.min(state.tasks.len() - 1);
    if was_active && !state.demo {
        fx.push(Effect::Activate(state.tasks[state.active].name.clone()));
    }
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
        assert_eq!(s.focus, Pane::Explorer);
    }

    #[test]
    fn compact_focus_cycles_through_four_panes() {
        let mut s = demo_state();
        s.compact = true;
        s.focus = Pane::Editor;
        update(&mut s, Action::FocusRight);
        update(&mut s, Action::FocusRight);
        assert_eq!(s.focus, Pane::Terminal);
        update(&mut s, Action::FocusRight);
        assert_eq!(s.focus, Pane::Explorer);
        update(&mut s, Action::FocusRight);
        assert_eq!(s.focus, Pane::Editor);
        update(&mut s, Action::FocusLeft);
        assert_eq!(s.focus, Pane::Explorer);
        update(&mut s, Action::FocusLeft);
        assert_eq!(s.focus, Pane::Terminal);
    }

    fn full_layout() -> AppState {
        let mut s = demo_state();
        s.set_layout(false, true);
        s
    }

    #[test]
    fn explorer_focus_neighbours() {
        let mut s = full_layout();
        s.focus = Pane::Editor;
        update(&mut s, Action::FocusLeft);
        assert_eq!(s.focus, Pane::Explorer);
        update(&mut s, Action::FocusLeft);
        assert_eq!(s.focus, Pane::Explorer);
        update(&mut s, Action::FocusRight);
        assert_eq!(s.focus, Pane::Editor);
        update(&mut s, Action::FocusLeft);
        update(&mut s, Action::FocusDown);
        assert_eq!(s.focus, Pane::Terminal);
        update(&mut s, Action::FocusUp);
        assert_eq!(s.focus, Pane::Editor);
    }

    #[test]
    fn hidden_explorer_is_not_a_focus_target() {
        let mut s = demo_state();
        s.set_layout(false, false);
        s.focus = Pane::Editor;
        update(&mut s, Action::FocusLeft);
        assert_eq!(s.focus, Pane::Editor);
    }

    #[test]
    fn toggling_explorer_hides_it_and_moves_focus() {
        let mut s = full_layout();
        assert!(s.explorer_visible());
        s.focus = Pane::Explorer;
        update(&mut s, Action::ToggleExplorer);
        assert!(!s.explorer_visible());
        assert_eq!(s.focus, Pane::Editor);
        update(&mut s, Action::ToggleExplorer);
        assert!(s.explorer_visible());
    }

    #[test]
    fn medium_tier_starts_hidden_and_toggle_shows_it() {
        let mut s = demo_state();
        s.set_layout(false, false);
        assert!(!s.explorer_visible());
        update(&mut s, Action::ToggleExplorer);
        assert!(s.explorer_visible());
    }

    #[test]
    fn resize_that_hides_explorer_moves_focus() {
        let mut s = full_layout();
        s.focus = Pane::Explorer;
        s.set_layout(false, false);
        assert_eq!(s.focus, Pane::Editor);
    }

    #[test]
    fn compact_alt_e_enters_and_leaves_the_explorer_tab() {
        let mut s = demo_state();
        s.set_layout(true, false);
        s.focus = Pane::Agent;
        update(&mut s, Action::ToggleExplorer);
        assert_eq!(s.focus, Pane::Explorer);
        update(&mut s, Action::ToggleExplorer);
        assert_eq!(s.focus, Pane::Editor);
    }

    fn explorer_focused() -> AppState {
        let mut s = full_layout();
        s.focus = Pane::Explorer;
        s
    }

    fn selected_path(s: &AppState) -> String {
        let t = s.active_task().expect("tarea activa");
        let rows = visible_rows(&t.files, &t.explorer.expanded);
        rows[t.explorer.selected].path.clone()
    }

    #[test]
    fn explorer_selection_moves_and_clamps() {
        let mut s = explorer_focused();
        let n = {
            let t = &s.tasks[0];
            visible_rows(&t.files, &t.explorer.expanded).len()
        };
        s.tasks[0].explorer.selected = 0;
        press(&mut s, KeyCode::Char('k'));
        assert_eq!(s.tasks[0].explorer.selected, 0);
        press(&mut s, KeyCode::Char('j'));
        press(&mut s, KeyCode::Down);
        assert_eq!(s.tasks[0].explorer.selected, 2);
        press(&mut s, KeyCode::Up);
        assert_eq!(s.tasks[0].explorer.selected, 1);
        for _ in 0..(n + 5) {
            press(&mut s, KeyCode::Char('j'));
        }
        assert_eq!(s.tasks[0].explorer.selected, n - 1);
    }

    #[test]
    fn explorer_enter_and_l_toggle_dirs_h_collapses() {
        let mut s = explorer_focused();
        s.tasks[0].explorer.expanded.clear();
        s.tasks[0].explorer.selected = 0;
        let dir = selected_path(&s);
        press(&mut s, KeyCode::Enter);
        assert!(s.tasks[0].explorer.expanded.contains(&dir));
        assert_eq!(s.focus, Pane::Explorer);
        press(&mut s, KeyCode::Char('l'));
        assert!(!s.tasks[0].explorer.expanded.contains(&dir));
        press(&mut s, KeyCode::Char('l'));
        assert!(s.tasks[0].explorer.expanded.contains(&dir));
        press(&mut s, KeyCode::Char('h'));
        assert!(!s.tasks[0].explorer.expanded.contains(&dir));
        assert_eq!(selected_path(&s), dir);
    }

    #[test]
    fn explorer_h_on_file_or_collapsed_dir_goes_to_parent() {
        let mut s = explorer_focused();
        let t = &mut s.tasks[0];
        t.explorer.expanded.clear();
        t.explorer.expanded.insert("src".to_string());
        t.explorer.expanded.insert("src/util".to_string());
        let rows = visible_rows(&t.files, &t.explorer.expanded);
        t.explorer.selected = rows
            .iter()
            .position(|r| r.path == "src/util/mod.rs")
            .expect("fila");
        press(&mut s, KeyCode::Char('h'));
        assert_eq!(selected_path(&s), "src/util");
        press(&mut s, KeyCode::Char('h')); // expandida: se contrae
        assert_eq!(selected_path(&s), "src/util");
        press(&mut s, KeyCode::Char('h')); // contraída: sube a src
        assert_eq!(selected_path(&s), "src");
        press(&mut s, KeyCode::Char('h')); // src expandida: se contrae
        press(&mut s, KeyCode::Char('h')); // raíz: sin padre, no se mueve
        assert_eq!(selected_path(&s), "src");
        assert!(!s.tasks[0].explorer.expanded.contains("src"));
    }

    #[test]
    fn explorer_enter_on_file_opens_in_editor() {
        let mut s = explorer_focused();
        let t = &mut s.tasks[0];
        t.explorer.expanded.insert("src".to_string());
        let rows = visible_rows(&t.files, &t.explorer.expanded);
        t.explorer.selected = rows
            .iter()
            .position(|r| r.path == "src/config.rs")
            .expect("fila");
        press(&mut s, KeyCode::Enter);
        assert_eq!(s.tasks[0].editor_file.as_deref(), Some("src/config.rs"));
        assert_eq!(s.focus, Pane::Editor);
        assert_eq!(
            s.toasts.last().map(|t| t.text.as_str()),
            Some("DEMO: se abriría src/config.rs en nvim (--remote)")
        );
    }

    #[test]
    fn explorer_r_shows_refresh_toast() {
        let mut s = explorer_focused();
        press(&mut s, KeyCode::Char('r'));
        assert_eq!(
            s.toasts.last().map(|t| t.text.as_str()),
            Some("DEMO: árbol refrescado")
        );
    }

    #[test]
    fn explorer_keys_are_ignored_under_a_modal() {
        let mut s = explorer_focused();
        update(&mut s, Action::Help);
        let before = s.tasks[0].explorer.selected;
        press(&mut s, KeyCode::Char('j'));
        assert_eq!(s.tasks[0].explorer.selected, before);
    }

    #[test]
    fn explorer_keys_with_no_files_do_not_panic() {
        let mut s = explorer_focused();
        s.tasks[0].files.clear();
        for c in ['j', 'k', 'h', 'l', 'r'] {
            press(&mut s, KeyCode::Char(c));
        }
        press(&mut s, KeyCode::Enter);
        assert_eq!(s.focus, Pane::Explorer);
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

    /// Pasa una tecla por `map_key` y `update`, como el bucle de eventos.
    fn send(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
        let action = crate::keymap::map_key(state, key);
        update(state, action)
    }

    fn ctrl_space() -> KeyEvent {
        KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL)
    }

    fn plain(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn prefix_then_q_quits() {
        let mut s = demo_state();
        assert!(send(&mut s, ctrl_space()).is_empty());
        assert!(s.prefix_pending);
        assert_eq!(send(&mut s, plain('q')), vec![Effect::Quit]);
        assert!(s.should_quit);
        assert!(!s.prefix_pending);
    }

    #[test]
    fn prefix_then_digit_selects_task() {
        let mut s = demo_state();
        send(&mut s, ctrl_space());
        send(&mut s, plain('2'));
        assert_eq!(s.active, 1);
        assert!(!s.prefix_pending);
    }

    #[test]
    fn prefix_esc_cancels_without_action() {
        let mut s = demo_state();
        let before = s.clone();
        send(&mut s, ctrl_space());
        send(&mut s, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(s, before);
    }

    #[test]
    fn prefix_twice_is_literal_with_info_toast() {
        let mut s = demo_state();
        send(&mut s, ctrl_space());
        send(&mut s, ctrl_space());
        assert!(!s.prefix_pending);
        let t = s.toasts.last().expect("aviso");
        assert_eq!(t.text, "DEMO: se enviaría Ctrl+Space al panel");
        assert_eq!(t.level, ToastLevel::Info);
    }

    #[test]
    fn prefix_unknown_warns_and_cancels() {
        let mut s = demo_state();
        send(&mut s, ctrl_space());
        send(&mut s, plain('x'));
        assert!(!s.prefix_pending);
        let t = s.toasts.last().expect("aviso");
        assert_eq!(t.text, "Atajo desconocido: Ctrl+Space x");
        assert_eq!(t.level, ToastLevel::Warning);
    }

    #[test]
    fn nul_variant_is_a_prefix() {
        let mut s = demo_state();
        send(
            &mut s,
            KeyEvent::new(KeyCode::Char('@'), KeyModifiers::CONTROL),
        );
        assert!(s.prefix_pending);
    }

    #[test]
    fn tick_keeps_the_prefix_pending() {
        let mut s = demo_state();
        send(&mut s, ctrl_space());
        update(&mut s, Action::Tick);
        update(&mut s, Action::Resize);
        assert!(s.prefix_pending);
    }

    #[test]
    fn prefix_q_quits_from_any_pane_and_modal() {
        let modals = [
            Modal::None,
            Modal::Help,
            Modal::NewTask {
                name: String::new(),
                agent_idx: 0,
                base: "main".to_string(),
                field: 0,
            },
            Modal::Confirm {
                message: "¿?".to_string(),
                action: ConfirmAction::DiscardTask(0),
            },
            Modal::Review {
                files: fake_changes(),
                selected: 0,
            },
        ];
        for modal in modals {
            for focus in [Pane::Explorer, Pane::Editor, Pane::Agent, Pane::Terminal] {
                let mut s = demo_state();
                s.modal = modal.clone();
                s.focus = focus;
                send(&mut s, ctrl_space());
                assert_eq!(send(&mut s, plain('q')), vec![Effect::Quit], "{modal:?}");
            }
        }
    }

    #[test]
    fn plain_q_in_chat_input_is_text() {
        let mut s = demo_state();
        s.focus = Pane::Agent;
        let before = s.active_task().expect("tarea").input.clone();
        assert!(send(&mut s, plain('q')).is_empty());
        assert_eq!(s.active_task().expect("tarea").input, format!("{before}q"));
    }

    #[test]
    fn alt_shortcuts_still_work() {
        let mut s = demo_state();
        let alt = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT);
        send(&mut s, alt('2'));
        assert_eq!(s.active, 1);
        assert_eq!(send(&mut s, alt('q')), vec![Effect::Quit]);
    }

    #[test]
    fn shortcut_label_per_action_and_style() {
        use crate::shortcuts::shortcut_label;
        use crate::state::ShortcutStyle::{Direct, Prefix};
        let (cmd, ctrl) = (KeyModifiers::SUPER, KeyModifiers::CONTROL);
        let cases = [
            (Direct, cmd, "review", "Cmd+Alt+R"),
            (Direct, ctrl, "review", "Ctrl+Alt+R"),
            (Direct, ctrl, "toggle_explorer", "Ctrl+B"),
            (Direct, cmd, "toggle_terminal", "Cmd+J"),
            (Direct, cmd, "new_task", "Cmd+Alt+N"),
            (Direct, ctrl, "quit", "Ctrl+Alt+Q"),
            (Direct, cmd, "select_task", "Cmd+1..9"),
            (Direct, cmd, "focus_left", "Cmd+Alt+←"),
            (Direct, ctrl, "focus_down", "Ctrl+Alt+↓"),
            (Direct, ctrl, "focus_up", "Ctrl+Alt+↑"),
            (Direct, ctrl, "focus_right", "Ctrl+Alt+→"),
            (Direct, cmd, "help", "F1"),
            // Sin atajo directo: siempre el prefijo.
            (Direct, cmd, "zoom", "Ctrl+Space z"),
            (Direct, ctrl, "last_toast", "Ctrl+Space m"),
            (Prefix, cmd, "review", "Ctrl+Space d"),
            (Prefix, ctrl, "toggle_explorer", "Ctrl+Space e"),
            (Prefix, ctrl, "select_task", "Ctrl+Space 1..9"),
            (Prefix, cmd, "help", "Ctrl+Space ?"),
        ];
        for (style, m, id, want) in cases {
            assert_eq!(shortcut_label(style, m, id), want, "{id} {style:?} {m:?}");
        }
    }
}

#[cfg(test)]
mod mouse_tests {
    use super::*;
    use crate::demo::demo_state;
    use crate::view;

    fn sized(w: u16, h: u16) -> AppState {
        let mut s = demo_state();
        s.size = (w, h);
        let tier = view::tier(w, h);
        s.set_layout(tier == Tier::Compact, tier == Tier::Full);
        s
    }

    fn areas(s: &AppState) -> Areas {
        layout::compute(Rect::new(0, 0, s.size.0, s.size.1), s)
    }

    fn mouse(kind: MouseEventKind, x: u16, y: u16) -> Action {
        Action::Mouse(MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn click(s: &mut AppState, x: u16, y: u16) {
        update(s, mouse(MouseEventKind::Down(MouseButton::Left), x, y));
    }

    fn wheel_at(s: &mut AppState, x: u16, y: u16, up: bool) {
        let kind = if up {
            MouseEventKind::ScrollUp
        } else {
            MouseEventKind::ScrollDown
        };
        update(s, mouse(kind, x, y));
    }

    fn center(r: Rect) -> (u16, u16) {
        (r.x + r.width / 2, r.y + r.height / 2)
    }

    fn chat_of(n: usize) -> Vec<ChatItem> {
        (0..n)
            .map(|i| ChatItem::Agent(format!("mensaje {i}")))
            .collect()
    }

    #[test]
    fn click_on_each_pane_focuses_it() {
        let mut s = sized(120, 35);
        let a = areas(&s);
        assert_eq!(a.panes().len(), 4);
        for (pane, rect) in a.panes() {
            // Borde superior: no cae sobre ninguna fila del explorador ni opción de permiso.
            s.focus = if pane == Pane::Editor {
                Pane::Agent
            } else {
                Pane::Editor
            };
            click(&mut s, rect.x + 1, rect.y);
            assert_eq!(s.focus, pane);
        }
        // Centro de los paneles que no tienen filas clicables.
        for pane in [Pane::Editor, Pane::Terminal, Pane::Agent] {
            s.focus = Pane::Explorer;
            let rect = a
                .panes()
                .into_iter()
                .find(|(p, _)| *p == pane)
                .expect("panel")
                .1;
            let (x, y) = center(rect);
            click(&mut s, x, y);
            assert_eq!(s.focus, pane);
        }
    }

    #[test]
    fn click_on_minimized_terminal_focuses_it() {
        let mut s = sized(100, 30);
        let a = areas(&s);
        assert!(a.terminal_minimized);
        let (x, y) = center(a.terminal.expect("terminal"));
        click(&mut s, x, y);
        assert_eq!(s.focus, Pane::Terminal);
    }

    #[test]
    fn click_on_zoomed_pane_keeps_focus() {
        let mut s = sized(120, 35);
        s.zoom = true;
        s.focus = Pane::Terminal;
        let a = areas(&s);
        assert_eq!(a.panes().len(), 1);
        let (x, y) = center(a.body);
        click(&mut s, x, y);
        assert_eq!(s.focus, Pane::Terminal);
    }

    #[test]
    fn click_on_task_selects_it_and_clears_done() {
        let mut s = sized(120, 35);
        s.tasks[2].status = AgentStatus::Done;
        let a = areas(&s);
        assert_eq!(a.task_tabs.len(), 3);
        let (x, y) = center(a.task_tabs[1].rect);
        click(&mut s, x, y);
        assert_eq!(s.active, 1);
        let (x, y) = center(a.task_tabs[2].rect);
        click(&mut s, x, y);
        assert_eq!(s.active, 2);
        assert_eq!(s.tasks[2].status, AgentStatus::Idle);
        // Los huecos entre tareas no hacen nada.
        let gap = a.task_tabs[0].rect.right();
        click(&mut s, gap, 0);
        assert_eq!(s.active, 2);
    }

    #[test]
    fn compact_tab_clicks_focus_each_pane() {
        let mut s = sized(80, 24);
        assert!(s.compact);
        let a = areas(&s);
        assert_eq!(a.compact_tabs.len(), 4);
        for (rect, pane) in a.compact_tabs.clone() {
            let (x, y) = center(rect);
            click(&mut s, x, y);
            assert_eq!(s.focus, pane);
        }
        // Estrecho (< 48 columnas): pestañas sin holgura.
        let mut s = sized(44, 12);
        let a = areas(&s);
        assert_eq!(a.compact_tabs.len(), 4);
        let (rect, pane) = a.compact_tabs[3];
        click(&mut s, rect.x, rect.y);
        assert_eq!(s.focus, pane);
    }

    #[test]
    fn compact_body_click_focuses_the_visible_pane() {
        let mut s = sized(80, 24);
        s.focus = Pane::Editor;
        let a = areas(&s);
        assert!(a.agent.is_none() && a.explorer.is_none() && a.terminal.is_none());
        let (x, y) = center(a.editor.expect("editor"));
        click(&mut s, x, y);
        assert_eq!(s.focus, Pane::Editor);
    }

    fn explorer_inner_and_start(s: &AppState) -> (Rect, usize) {
        let inner = layout::inner(areas(s).explorer.expect("explorador"));
        let task = s.active_task().expect("tarea");
        let rows = visible_rows(&task.files, &task.explorer.expanded);
        let sel = task.explorer.selected.min(rows.len() - 1);
        (
            inner,
            explorer_view::scroll_start(sel, usize::from(inner.height)),
        )
    }

    fn rows_of(s: &AppState) -> Vec<crate::explorer::Row> {
        let t = s.active_task().expect("tarea");
        visible_rows(&t.files, &t.explorer.expanded)
    }

    #[test]
    fn explorer_click_selects_then_activates_dir() {
        let mut s = sized(120, 35);
        s.focus = Pane::Editor;
        let (inner, start) = explorer_inner_and_start(&s);
        let rows = rows_of(&s);
        let dir = rows.iter().position(|r| r.is_dir()).expect("carpeta");
        let path = rows[dir].path.clone();
        let y = inner.y + (dir - start) as u16;
        let was_expanded = s.tasks[0].explorer.expanded.contains(&path);
        click(&mut s, inner.x + 2, y);
        assert_eq!(s.focus, Pane::Explorer);
        assert_eq!(s.tasks[0].explorer.selected, dir);
        assert_eq!(s.tasks[0].explorer.expanded.contains(&path), was_expanded);
        click(&mut s, inner.x + 2, y);
        assert_eq!(s.tasks[0].explorer.expanded.contains(&path), !was_expanded);
        assert_eq!(s.focus, Pane::Explorer);
    }

    #[test]
    fn explorer_click_on_file_opens_it_like_enter() {
        let mut s = sized(120, 35);
        let (inner, start) = explorer_inner_and_start(&s);
        let rows = rows_of(&s);
        let file = rows.iter().rposition(|r| !r.is_dir()).expect("archivo");
        let y = inner.y + (file - start) as u16;
        click(&mut s, inner.x + 2, y);
        assert_eq!(s.focus, Pane::Explorer);
        click(&mut s, inner.x + 2, y);
        assert_eq!(s.focus, Pane::Editor);
        assert_eq!(
            s.tasks[0].editor_file.as_deref(),
            Some(rows[file].path.as_str())
        );
        assert!(s
            .toasts
            .iter()
            .any(|t| t.text.starts_with("DEMO: se abriría")));
    }

    #[test]
    fn explorer_click_below_last_row_only_focuses() {
        let mut s = sized(120, 35);
        s.focus = Pane::Editor;
        let before = s.tasks[0].explorer.clone();
        let inner = layout::inner(areas(&s).explorer.expect("explorador"));
        assert!(rows_of(&s).len() < usize::from(inner.height));
        click(&mut s, inner.x + 1, inner.bottom() - 1);
        assert_eq!(s.focus, Pane::Explorer);
        assert_eq!(s.tasks[0].explorer, before);
    }

    #[test]
    fn explorer_wheel_moves_selection_and_clamps() {
        let mut s = sized(120, 35);
        let (x, y) = center(areas(&s).explorer.expect("explorador"));
        let start = s.tasks[0].explorer.selected;
        wheel_at(&mut s, x, y, false);
        assert_eq!(s.tasks[0].explorer.selected, start + 1);
        wheel_at(&mut s, x, y, true);
        wheel_at(&mut s, x, y, true);
        assert_eq!(s.tasks[0].explorer.selected, start.saturating_sub(1));
        for _ in 0..50 {
            wheel_at(&mut s, x, y, true);
        }
        assert_eq!(s.tasks[0].explorer.selected, 0);
        for _ in 0..500 {
            wheel_at(&mut s, x, y, false);
        }
        assert_eq!(s.tasks[0].explorer.selected, rows_of(&s).len() - 1);
    }

    fn with_permission() -> AppState {
        let mut s = sized(120, 35);
        update(&mut s, Action::DemoSetStatus(AgentStatus::NeedsPermission));
        s
    }

    fn chat_view_of(s: &AppState) -> chat::ChatView {
        let rect = areas(s).agent.expect("agente");
        chat::chat_view(
            s.active_task().expect("tarea"),
            layout::inner(rect),
            s.ascii,
        )
    }

    #[test]
    fn permission_option_click_selects_then_resolves() {
        let mut s = with_permission();
        s.focus = Pane::Editor;
        let r = chat_view_of(&s)
            .options
            .iter()
            .find(|(_, i)| *i == 2)
            .map(|(r, _)| *r)
            .expect("opción visible");
        click(&mut s, r.x + 4, r.y);
        assert_eq!(s.focus, Pane::Agent);
        let perm = |s: &AppState| match s.tasks[0].chat.last() {
            Some(ChatItem::Permission {
                selected, resolved, ..
            }) => (*selected, *resolved),
            other => panic!("se esperaba un permiso, hay {other:?}"),
        };
        assert_eq!(perm(&s), (2, None));
        click(&mut s, r.x + 4, r.y);
        assert_eq!(perm(&s), (2, Some(2)));
        assert_eq!(s.tasks[0].status, AgentStatus::Working);
        assert!(s
            .toasts
            .iter()
            .any(|t| t.text == "Permiso respondido: Rechazar"));
    }

    #[test]
    fn click_on_agent_input_only_focuses() {
        let mut s = with_permission();
        s.focus = Pane::Editor;
        let (x, y) = center(chat_view_of(&s).input);
        click(&mut s, x, y);
        assert_eq!(s.focus, Pane::Agent);
        assert!(matches!(
            s.tasks[0].chat.last(),
            Some(ChatItem::Permission {
                resolved: None,
                selected: 0,
                ..
            })
        ));
    }

    #[test]
    fn chat_wheel_scrolls_by_three_clamps_and_resets_on_typing() {
        let mut s = sized(120, 35);
        s.tasks[0].chat.extend(chat_of(40));
        let (x, y) = center(areas(&s).agent.expect("agente"));
        wheel_at(&mut s, x, y, true);
        assert_eq!(s.tasks[0].chat_scroll, 3);
        wheel_at(&mut s, x, y, true);
        assert_eq!(s.tasks[0].chat_scroll, 6);
        wheel_at(&mut s, x, y, false);
        assert_eq!(s.tasks[0].chat_scroll, 3);
        let max = chat_view_of(&s).max_scroll;
        assert!(max > 6);
        for _ in 0..200 {
            wheel_at(&mut s, x, y, true);
        }
        assert_eq!(s.tasks[0].chat_scroll, max);
        for _ in 0..200 {
            wheel_at(&mut s, x, y, false);
        }
        assert_eq!(s.tasks[0].chat_scroll, 0);
        wheel_at(&mut s, x, y, true);
        s.focus = Pane::Agent;
        press(&mut s, KeyCode::Char('a'));
        assert_eq!(s.tasks[0].chat_scroll, 0);
        wheel_at(&mut s, x, y, true);
        press(&mut s, KeyCode::Enter);
        assert_eq!(s.tasks[0].chat_scroll, 0);
    }

    fn press(s: &mut AppState, code: KeyCode) {
        update(s, Action::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    #[test]
    fn options_scrolled_out_of_view_are_not_clickable() {
        let mut s = with_permission();
        assert_eq!(chat_view_of(&s).options.len(), 3);
        s.tasks[0].chat.extend(chat_of(60));
        assert!(chat_view_of(&s).options.is_empty());
    }

    #[test]
    fn wheel_over_editor_and_terminal_is_ignored() {
        let mut s = sized(120, 35);
        let before = s.clone();
        let a = areas(&s);
        for rect in [a.editor, a.terminal].into_iter().flatten() {
            let (x, y) = center(rect);
            wheel_at(&mut s, x, y, true);
            wheel_at(&mut s, x, y, false);
        }
        assert_eq!(s, before);
    }

    #[test]
    fn help_closes_on_outside_click_only() {
        let mut s = sized(120, 35);
        s.modal = Modal::Help;
        let a = areas(&s);
        let rect = modals::help_rect(a.body);
        let (x, y) = center(rect);
        click(&mut s, x, y);
        assert_eq!(s.modal, Modal::Help);
        click(&mut s, rect.x, rect.y);
        assert_eq!(s.modal, Modal::Help);
        // Fuera del diálogo (sobre el explorador): cierra sin cambiar el foco.
        click(&mut s, 0, a.body.y);
        assert_eq!(s.modal, Modal::None);
        assert_eq!(s.focus, Pane::Agent);
    }

    #[test]
    fn new_task_and_confirm_ignore_the_mouse() {
        for modal in [
            Modal::NewTask {
                name: "abc".to_string(),
                agent_idx: 0,
                base: "main".to_string(),
                field: 0,
            },
            Modal::Confirm {
                message: "¿?".to_string(),
                action: ConfirmAction::DiscardTask(0),
            },
        ] {
            let mut s = sized(120, 35);
            s.modal = modal;
            let before = s.clone();
            click(&mut s, 0, 0);
            click(&mut s, 5, 0);
            click(&mut s, 60, 17);
            wheel_at(&mut s, 60, 17, true);
            assert_eq!(s, before);
        }
    }

    #[test]
    fn review_click_selects_file_and_outside_closes() {
        let mut s = sized(120, 35);
        s.modal = Modal::Review {
            files: fake_changes(),
            selected: 0,
        };
        let a = areas(&s);
        let zones = review::areas(a.body);
        let list = layout::inner(zones.list);
        click(&mut s, list.x + 1, list.y + 2);
        assert!(matches!(s.modal, Modal::Review { selected: 2, .. }));
        // Debajo del último archivo no cambia nada.
        click(&mut s, list.x + 1, list.y + 10);
        assert!(matches!(s.modal, Modal::Review { selected: 2, .. }));
        // El diff no selecciona.
        click(&mut s, zones.diff.x + 3, zones.diff.y + 3);
        assert!(matches!(s.modal, Modal::Review { selected: 2, .. }));
        wheel_at(&mut s, list.x + 1, list.y, true);
        assert!(matches!(s.modal, Modal::Review { selected: 1, .. }));
        for _ in 0..10 {
            wheel_at(&mut s, list.x + 1, list.y, false);
        }
        assert!(matches!(s.modal, Modal::Review { selected: 2, .. }));
        click(&mut s, 0, a.taskbar.y);
        assert_eq!(s.modal, Modal::None);
    }

    #[test]
    fn modal_blocks_clicks_on_tasks() {
        let mut s = sized(120, 35);
        s.modal = Modal::Help;
        let (x, y) = center(areas(&s).task_tabs[1].rect);
        click(&mut s, x, y);
        // Cierra la ayuda (clic fuera) pero no cambia de tarea.
        assert_eq!(s.modal, Modal::None);
        assert_eq!(s.active, 0);
    }

    #[test]
    fn clicks_far_outside_or_on_tiny_screens_change_nothing() {
        let mut s = sized(120, 35);
        let before = s.clone();
        click(&mut s, 999, 999);
        click(&mut s, u16::MAX, u16::MAX);
        wheel_at(&mut s, 999, 999, true);
        assert_eq!(s, before);

        let mut s = sized(10, 5);
        let before = s.clone();
        click(&mut s, 0, 0);
        click(&mut s, 5, 2);
        wheel_at(&mut s, 5, 2, true);
        assert_eq!(s, before);

        // Sin tamaño todavía (antes del primer dibujo).
        let mut s = demo_state();
        let before = s.clone();
        click(&mut s, 0, 0);
        assert_eq!(s, before);
    }

    #[test]
    fn moves_releases_and_other_buttons_do_nothing() {
        let mut s = sized(120, 35);
        let before = s.clone();
        let (x, y) = center(areas(&s).editor.expect("editor"));
        for kind in [
            MouseEventKind::Moved,
            MouseEventKind::Up(MouseButton::Left),
            MouseEventKind::Down(MouseButton::Right),
            MouseEventKind::ScrollLeft,
        ] {
            update(&mut s, mouse(kind, x, y));
        }
        assert_eq!(s, before);
    }
}

/// Comportamiento de la app real: `update` emite efectos y aplica los eventos del runtime.
#[cfg(test)]
mod real_tests {
    use super::*;
    use crate::demo::demo_state;
    use crate::layout;
    use crate::state::FileEntry;
    use qorvi_acp::{
        Cost, Mode, Modes, PermissionKind, PermissionOption, PermissionRequest, SessionReady,
        ToolCallInfo, ToolCallPatch, Usage,
    };

    fn real() -> AppState {
        let mut s = demo_state();
        s.demo = false;
        s.active = 0;
        s.focus = Pane::Agent;
        s.tasks[0].status = AgentStatus::Idle;
        s.tasks[0].input.clear();
        s
    }

    fn key(code: KeyCode) -> Action {
        Action::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ev(s: &mut AppState, name: &str, e: AgentEvent) -> Vec<Effect> {
        update(s, Action::Agent(name.to_string(), e))
    }

    fn type_and_send(s: &mut AppState, text: &str) -> Vec<Effect> {
        for c in text.chars() {
            update(s, key(KeyCode::Char(c)));
        }
        update(s, key(KeyCode::Enter))
    }

    #[test]
    fn prompt_emite_efecto_y_bloquea_otro_mientras_trabaja() {
        let mut s = real();
        let fx = type_and_send(&mut s, "hola");
        assert_eq!(fx, vec![Effect::Prompt("APP-12".into(), "hola".into())]);
        assert_eq!(s.tasks[0].status, AgentStatus::Working);
        assert_eq!(s.tasks[0].chat.last(), Some(&ChatItem::User("hola".into())));
        // Mientras trabaja, el texto se queda en la caja y se avisa.
        let fx = type_and_send(&mut s, "otra");
        assert!(fx.is_empty());
        assert_eq!(s.tasks[0].input, "otra");
        // Esc cancela.
        assert_eq!(
            update(&mut s, key(KeyCode::Esc)),
            vec![Effect::Cancel("APP-12".into())]
        );
    }

    #[test]
    fn trozos_se_acumulan_y_las_herramientas_se_actualizan() {
        let mut s = real();
        s.tasks[0].chat.clear();
        ev(&mut s, "APP-12", AgentEvent::MessageChunk("Hola, ".into()));
        ev(&mut s, "APP-12", AgentEvent::MessageChunk("mundo".into()));
        ev(
            &mut s,
            "APP-12",
            AgentEvent::ToolCall(ToolCallInfo {
                id: "t1".into(),
                title: "Leer".into(),
                kind: "read".into(),
                status: qorvi_acp::ToolStatus::Pending,
                diffs: vec![],
                text: vec![],
            }),
        );
        ev(
            &mut s,
            "APP-12",
            AgentEvent::ToolCallUpdate(ToolCallPatch {
                id: "t1".into(),
                title: Some("Leer a.rs".into()),
                kind: None,
                status: Some(qorvi_acp::ToolStatus::Completed),
                diffs: None,
                text: None,
            }),
        );
        ev(&mut s, "APP-12", AgentEvent::MessageChunk("fin".into()));
        let chat = &s.tasks[0].chat;
        assert_eq!(chat[0], ChatItem::Agent("Hola, mundo".into()));
        assert!(
            matches!(&chat[1], ChatItem::ToolCall { title, status: ToolStatus::Completed, .. } if title == "Leer a.rs")
        );
        assert_eq!(chat[2], ChatItem::Agent("fin".into()));
    }

    fn permission(id: u64) -> AgentEvent {
        AgentEvent::PermissionRequest(PermissionRequest {
            id,
            tool_call_id: "t".into(),
            title: "Ejecutar ls".into(),
            options: vec![
                PermissionOption {
                    id: "ok".into(),
                    name: "Permitir".into(),
                    kind: PermissionKind::AllowOnce,
                },
                PermissionOption {
                    id: "no".into(),
                    name: "Rechazar".into(),
                    kind: PermissionKind::RejectOnce,
                },
            ],
        })
    }

    #[test]
    fn permiso_se_responde_con_el_id_de_la_opcion() {
        let mut s = real();
        s.tasks[0].status = AgentStatus::Working;
        ev(&mut s, "APP-12", permission(7));
        assert_eq!(s.tasks[0].status, AgentStatus::NeedsPermission);
        update(&mut s, key(KeyCode::Down));
        let fx = update(&mut s, key(KeyCode::Enter));
        assert_eq!(
            fx,
            vec![Effect::Permission("APP-12".into(), 7, Some("no".into()))]
        );
        assert_eq!(s.tasks[0].status, AgentStatus::Working);
    }

    #[test]
    fn permiso_cancelado_queda_descartado() {
        let mut s = real();
        ev(&mut s, "APP-12", permission(3));
        ev(&mut s, "APP-12", AgentEvent::PermissionCancelled(3));
        assert!(matches!(
            s.tasks[0].chat.last(),
            Some(ChatItem::Permission {
                resolved: Some(2),
                ..
            })
        ));
        // Enter ya no responde a nada: con la caja vacía no emite efectos.
        assert!(update(&mut s, key(KeyCode::Enter)).is_empty());
    }

    #[test]
    fn tarea_en_segundo_plano_avisa_y_queda_lista() {
        let mut s = real();
        s.tasks[1].status = AgentStatus::Working;
        let name = s.tasks[1].name.clone();
        let fx = ev(&mut s, &name, AgentEvent::TurnEnded(StopReason::EndTurn));
        assert_eq!(fx, vec![Effect::RefreshFiles(name.clone())]);
        assert_eq!(s.tasks[1].status, AgentStatus::Done);
        assert!(s.toasts.iter().any(|t| t.text.contains(&name)));
        // Un permiso en segundo plano también avisa.
        s.toasts.clear();
        ev(&mut s, &name, permission(1));
        assert_eq!(s.toasts[0].level, ToastLevel::Warning);
        // La tarea activa no avisa al terminar y queda inactiva.
        s.toasts.clear();
        s.tasks[0].status = AgentStatus::Working;
        ev(
            &mut s,
            "APP-12",
            AgentEvent::TurnEnded(StopReason::Cancelled),
        );
        assert_eq!(s.tasks[0].status, AgentStatus::Idle);
        assert!(s.toasts.is_empty());
        assert_eq!(
            s.tasks[0].chat.last(),
            Some(&ChatItem::Notice("Turno cancelado".into()))
        );
    }

    #[test]
    fn sesion_coste_y_titulo_se_registran() {
        let mut s = real();
        s.tasks[0].status = AgentStatus::Connecting;
        let fx = ev(
            &mut s,
            "APP-12",
            AgentEvent::Ready(SessionReady {
                session_id: "s1".into(),
                resumed: true,
                agent: None,
                modes: Some(Modes {
                    current: "default".into(),
                    available: vec![
                        Mode {
                            id: "default".into(),
                            name: "Preguntar".into(),
                            description: None,
                        },
                        Mode {
                            id: "plan".into(),
                            name: "Planificar".into(),
                            description: None,
                        },
                    ],
                }),
            }),
        );
        assert_eq!(s.tasks[0].status, AgentStatus::Idle);
        assert!(
            matches!(&fx[0], Effect::RecordSession(n, i) if n == "APP-12" && i.session_id.as_deref() == Some("s1"))
        );
        let fx = ev(
            &mut s,
            "APP-12",
            AgentEvent::Usage(Usage {
                used: 1,
                size: 2,
                cost: Some(Cost {
                    amount: 1.5,
                    currency: "USD".into(),
                }),
            }),
        );
        assert_eq!(s.tasks[0].cost_usd, 1.5);
        assert!(
            matches!(&fx[0], Effect::RecordSession(_, i) if i.cost.as_ref().is_some_and(|c| c.amount == 1.5))
        );
        let fx = ev(&mut s, "APP-12", AgentEvent::Title(Some("Login".into())));
        assert_eq!(s.tasks[0].title.as_deref(), Some("Login"));
        assert!(
            matches!(&fx[0], Effect::RecordSession(_, i) if i.title == Some(Some("Login".into())))
        );
        // Shift+Tab pasa al siguiente modo.
        assert_eq!(
            update(&mut s, key(KeyCode::BackTab)),
            vec![Effect::SetMode("APP-12".into(), "plan".into())]
        );
        ev(&mut s, "APP-12", AgentEvent::ModeChanged("plan".into()));
        assert_eq!(s.tasks[0].mode.as_deref(), Some("plan"));
    }

    #[test]
    fn agente_que_se_cierra_con_error() {
        let mut s = real();
        ev(&mut s, "APP-12", permission(1));
        ev(
            &mut s,
            "APP-12",
            AgentEvent::Exited {
                code: Some(1),
                error: Some("adiós".into()),
            },
        );
        assert_eq!(s.tasks[0].status, AgentStatus::Error);
        assert!(last_unresolved_permission(&s.tasks[0]).is_none());
    }

    #[test]
    fn teclas_del_editor_van_al_panel_o_reinician() {
        let mut s = real();
        s.focus = Pane::Editor;
        let k = KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE);
        assert_eq!(
            update(&mut s, Action::Key(k)),
            vec![Effect::PaneInput(
                "APP-12".into(),
                Pane::Editor,
                PaneInput::Key(k)
            )]
        );
        s.tasks[0].editor_note = Some("nvim terminó".into());
        assert!(update(&mut s, Action::Key(k)).is_empty());
        assert_eq!(
            update(&mut s, key(KeyCode::Enter)),
            vec![Effect::RestartPane("APP-12".into(), Pane::Editor)]
        );
        // Ctrl+Space dos veces envía NUL al panel.
        s.tasks[0].editor_note = None;
        assert_eq!(
            update(&mut s, Action::PrefixLiteral),
            vec![Effect::PaneInput(
                "APP-12".into(),
                Pane::Editor,
                PaneInput::Bytes(vec![0])
            )]
        );
    }

    #[test]
    fn clic_y_rueda_en_el_editor_llegan_con_coordenadas_relativas() {
        let mut s = real();
        s.size = (140, 40);
        s.set_layout(false, true);
        let areas = layout::compute(Rect::new(0, 0, 140, 40), &s);
        let inner = layout::inner(areas.editor.unwrap());
        let m = |kind| MouseEvent {
            kind,
            column: inner.x + 2,
            row: inner.y + 1,
            modifiers: KeyModifiers::NONE,
        };
        let fx = update(
            &mut s,
            Action::Mouse(m(MouseEventKind::Down(MouseButton::Left))),
        );
        assert_eq!(s.focus, Pane::Editor);
        assert!(
            matches!(&fx[..], [Effect::PaneInput(_, Pane::Editor, PaneInput::Mouse(e))] if e.column == 2 && e.row == 1)
        );
        let fx = update(&mut s, Action::Mouse(m(MouseEventKind::ScrollUp)));
        assert!(matches!(
            &fx[..],
            [Effect::PaneInput(_, Pane::Editor, PaneInput::Wheel(_))]
        ));
    }

    #[test]
    fn seleccionar_crear_y_borrar_tareas() {
        let mut s = real();
        assert_eq!(
            update(&mut s, Action::SelectTask(1)),
            vec![Effect::Activate(s.tasks[1].name.clone())]
        );
        update(&mut s, Action::NewTask);
        let fx = type_and_send(&mut s, "nueva");
        assert_eq!(
            fx,
            vec![Effect::CreateTask {
                name: "nueva".into(),
                agent: "claude".into(),
                base: "main".into()
            }]
        );
        assert_eq!(s.modal, Modal::None);
        let fx = update(
            &mut s,
            Action::TaskCreated(
                Box::new(Task::new("nueva", "claude", "main")),
                vec!["ojo".into()],
            ),
        );
        assert_eq!(fx, vec![Effect::Activate("nueva".into())]);
        assert_eq!(s.active_task().unwrap().name, "nueva");
        assert!(s.toasts.iter().any(|t| t.text == "ojo"));

        // Review → descartar → confirmar pide el borrado; TaskRemoved lo aplica.
        let fx = update(&mut s, Action::OpenReview);
        assert_eq!(fx, vec![Effect::LoadReview("nueva".into())]);
        update(&mut s, Action::ReviewLoaded("nueva".into(), vec![]));
        assert!(matches!(s.modal, Modal::Review { .. }));
        assert_eq!(
            update(&mut s, key(KeyCode::Char('m'))),
            vec![Effect::Merge("nueva".into())]
        );
        update(&mut s, key(KeyCode::Char('x')));
        assert_eq!(
            update(&mut s, key(KeyCode::Enter)),
            vec![Effect::RemoveTask("nueva".into())]
        );
        let n = s.tasks.len();
        let fx = update(&mut s, Action::TaskRemoved("nueva".into()));
        assert_eq!(s.tasks.len(), n - 1);
        assert_eq!(
            fx,
            vec![Effect::Activate(s.active_task().unwrap().name.clone())]
        );
    }

    #[test]
    fn pegar_y_explorador() {
        let mut s = real();
        update(&mut s, Action::Paste("línea 1\r\nlínea 2".into()));
        assert_eq!(s.tasks[0].input, "línea 1\nlínea 2");
        s.focus = Pane::Explorer;
        s.tasks[0].files = vec![FileEntry {
            path: "a.rs".into(),
            mark: None,
        }];
        s.tasks[0].explorer.selected = 0;
        assert_eq!(
            update(&mut s, key(KeyCode::Enter)),
            vec![Effect::OpenFile("APP-12".into(), "a.rs".into())]
        );
        assert_eq!(s.focus, Pane::Editor);
        s.focus = Pane::Explorer;
        assert_eq!(
            update(&mut s, key(KeyCode::Char('r'))),
            vec![Effect::RefreshFiles("APP-12".into())]
        );
    }

    #[test]
    fn f5_solo_en_la_demo() {
        let s = real();
        let a = crate::keymap::map_key(&s, KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
        assert!(!matches!(a, Action::DemoSetStatus(_)));
    }
}

#[cfg(test)]
mod alt_tests {
    use super::*;
    use crate::demo::demo_state;

    #[test]
    fn avisa_una_vez_si_alt_llega_como_simbolo() {
        let mut s = demo_state();
        s.focus = Pane::Agent;
        let k = |c| Action::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        update(&mut s, k('¡'));
        update(&mut s, k('ñ'));
        assert!(s.toasts.is_empty());
        update(&mut s, k('˙'));
        update(&mut s, k('∆'));
        let avisos: Vec<_> = s
            .toasts
            .iter()
            .filter(|t| t.text.contains("Option"))
            .collect();
        assert_eq!(avisos.len(), 1);
        assert!(avisos[0].text.contains('˙'));
        // La tecla no se pierde: llega a la caja de texto.
        assert!(s.tasks[s.active].input.ends_with("¡ñ˙∆"));
    }
}
