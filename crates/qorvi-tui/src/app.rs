//! App real: bucle de eventos y runtime que ejecuta los [`Effect`] de `update` con el
//! workspace (git), los paneles (PTY) y los agentes (ACP).
//!
//! Todo lo que llega de fuera (teclado, salida de los paneles, eventos de los agentes)
//! entra por un único canal; el hilo principal lo convierte en acciones, llama a
//! `update` y ejecuta los efectos. Merge y push + PR, que pueden tardar, corren en un hilo
//! aparte (con un spinner en la barra de estado); el resto de git es rápido y síncrono.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};
use qorvi_acp::{AgentCommand, AgentEvent, AgentHandle, PermissionChoice, SessionSpec};
use qorvi_core::{
    FileState, Gh, Git, MergeStyle, NewTask, RemoveOptions, Task as CoreTask, Workspace,
};
use qorvi_pty::editor::{editor_command, open_file_keys, EditorKind, NvimRemote};
use qorvi_pty::{Pane as PtyPane, PaneCommand, PaneEvent, PaneSize};
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::DefaultTerminal;

use crate::action::{Action, PaneInput};
use crate::keymap;
use crate::layout;
use crate::state::{
    AgentStatus, AppState, ChangeKind, ChangeMark, ChangedFile, ChatItem, FileEntry, Pane, Task,
    ToastLevel,
};
use crate::update::{update, Effect};
use crate::view::{self, term::draw_screen, term::Screens, Tier};

const TICK: Duration = Duration::from_millis(250);
/// Historial de las terminales de las tareas.
const SCROLLBACK: usize = 5000;

enum Msg {
    Input(Event),
    Action(Action),
    Pane(String, Pane, PaneEvent),
}

/// Recursos vivos de una tarea.
struct TaskRt {
    task: CoreTask,
    agent: Option<AgentHandle>,
    editor: Option<PtyPane>,
    editor_kind: EditorKind,
    nvim: Option<NvimRemote>,
    terminal: Option<PtyPane>,
    /// `setup` de la config, para la primera terminal de una tarea recién creada.
    pending_setup: Option<String>,
}

impl TaskRt {
    fn new(task: CoreTask) -> TaskRt {
        TaskRt {
            task,
            agent: None,
            editor: None,
            editor_kind: EditorKind::Other,
            nvim: None,
            terminal: None,
            pending_setup: None,
        }
    }

    fn pane(&self, pane: Pane) -> Option<&PtyPane> {
        match pane {
            Pane::Editor => self.editor.as_ref(),
            Pane::Terminal => self.terminal.as_ref(),
            _ => None,
        }
    }
}

/// Estado inicial de la interfaz para el workspace.
pub fn initial_state(ws: &Workspace) -> qorvi_core::Result<AppState> {
    let config = ws.config();
    let mut agents: Vec<String> = config.agents.keys().cloned().collect();
    if let Some(def) = &config.default_agent {
        if let Some(i) = agents.iter().position(|a| a == def) {
            let d = agents.remove(i);
            agents.insert(0, d);
        }
    }
    let editor_name = config
        .editor_command()
        .and_then(|e| e.split_whitespace().next().map(program_name))
        .unwrap_or_else(|| "editor".to_string());
    let shell_name = program_name(&config.shell_command());
    let tasks: Vec<Task> = ws
        .tasks()?
        .iter()
        .map(|t| {
            let mut ui = ui_task(t);
            ui.chat = load_chat(&ws.paths().chat_file(t.name.as_str()));
            ui
        })
        .collect();
    let mut state = crate::demo::demo_state();
    state.demo = false;
    state.tasks = tasks;
    state.active = 0;
    state.focus = Pane::Agent;
    state.modal = crate::state::Modal::None;
    state.toasts.clear();
    state.last_toast = None;
    state.agents = agents;
    state.default_base = ws.default_base().unwrap_or_else(|_| "main".to_string());
    state.editor_name = editor_name;
    state.shell_name = shell_name;
    Ok(state)
}

fn program_name(cmd: &str) -> String {
    let first = cmd.split_whitespace().next().unwrap_or(cmd);
    let name = first.rsplit(['/', '\\']).next().unwrap_or(first);
    name.strip_suffix(".exe").unwrap_or(name).to_string()
}

/// Tarea de la interfaz a partir de la persistida.
fn ui_task(t: &CoreTask) -> Task {
    let mut ui = Task::new(t.name.as_str(), &t.agent, &t.base_ref);
    ui.branch = t.branch.clone();
    ui.status = AgentStatus::Disconnected;
    ui.title = t.title.clone();
    ui.cost_usd = t.cost.as_ref().map_or(0.0, |c| c.amount);
    ui.editor_lines.clear();
    ui.terminal_lines.clear();
    ui
}

/// Ejecuta la app hasta que el usuario sale.
pub fn run(terminal: &mut DefaultTerminal, mut state: AppState, ws: Workspace) -> io::Result<()> {
    let (tx, rx) = mpsc::channel();
    let input_tx = tx.clone();
    std::thread::Builder::new()
        .name("qorvi-input".into())
        .spawn(move || {
            while let Ok(ev) = event::read() {
                if input_tx.send(Msg::Input(ev)).is_err() {
                    break;
                }
            }
        })?;
    let mut rt = Runtime {
        tasks: ws
            .tasks()
            .map_err(io::Error::other)?
            .into_iter()
            .map(|t| (t.name.to_string(), TaskRt::new(t)))
            .collect(),
        ws,
        tx,
        sizes: Sizes::default(),
    };

    let mut queue: VecDeque<Action> = VecDeque::new();
    if let Some(t) = state.active_task() {
        let name = t.name.clone();
        rt.exec(Effect::Activate(name), &state, &mut queue);
    } else {
        queue.push_back(Action::NewTask);
    }
    let result = event_loop(terminal, &mut state, &mut rt, &rx, &mut queue);
    for t in &state.tasks {
        let _ = save_chat(&rt.ws.paths().chat_file(&t.name), &t.chat);
    }
    rt.shutdown();
    result
}

fn event_loop(
    terminal: &mut DefaultTerminal,
    state: &mut AppState,
    rt: &mut Runtime,
    rx: &Receiver<Msg>,
    queue: &mut VecDeque<Action>,
) -> io::Result<()> {
    let mut last_tick = Instant::now();
    loop {
        // Acciones pendientes (del runtime o de mensajes ya leídos).
        if drain(state, rt, queue) {
            return Ok(());
        }

        let size = terminal.size()?;
        let tier = view::tier(size.width, size.height);
        state.size = (size.width, size.height);
        state.set_layout(tier == Tier::Compact, tier == Tier::Full);
        rt.resize_active(state);
        terminal.draw(|frame| view::view_with(state, frame, &RtScreens(&rt.tasks)))?;

        let timeout = TICK.saturating_sub(last_tick.elapsed());
        let mut msgs = Vec::new();
        match rx.recv_timeout(timeout) {
            Ok(m) => msgs.push(m),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        // Agrupa lo que haya llegado para dibujar una sola vez (salida rápida de un panel).
        while msgs.len() < 512 {
            match rx.try_recv() {
                Ok(m) => msgs.push(m),
                Err(_) => break,
            }
        }
        for m in msgs {
            // Las teclas se traducen con el estado al día (el prefijo depende del anterior).
            if drain(state, rt, queue) {
                return Ok(());
            }
            if let Some(action) = rt.translate(state, m) {
                queue.push_back(action);
            }
        }
        if last_tick.elapsed() >= TICK {
            last_tick = Instant::now();
            queue.push_back(Action::Tick);
        }
    }
}

/// Aplica las acciones de la cola y ejecuta sus efectos. `true` si hay que salir.
fn drain(state: &mut AppState, rt: &mut Runtime, queue: &mut VecDeque<Action>) -> bool {
    while let Some(action) = queue.pop_front() {
        if let Action::Agent(name, AgentEvent::Exited { .. }) = &action {
            if let Some(t) = rt.tasks.get_mut(name) {
                t.agent = None;
            }
        }
        for effect in update(state, action) {
            if effect == Effect::Quit {
                return true;
            }
            rt.exec(effect, state, queue);
        }
    }
    false
}

#[derive(Default)]
struct Sizes {
    editor: Option<PaneSize>,
    terminal: Option<PaneSize>,
}

struct Runtime {
    ws: Workspace,
    tx: Sender<Msg>,
    tasks: HashMap<String, TaskRt>,
    sizes: Sizes,
}

struct RtScreens<'a>(&'a HashMap<String, TaskRt>);

impl Screens for RtScreens<'_> {
    fn draw(
        &self,
        task: &str,
        pane: Pane,
        area: Rect,
        buf: &mut Buffer,
    ) -> Option<Option<Position>> {
        let p = self.0.get(task)?.pane(pane)?;
        // `is_scrolled` y `with_screen` toman el mismo lock: primero uno, luego el otro.
        let live = !p.is_scrolled();
        Some(p.with_screen(|s| draw_screen(s, area, buf, live)))
    }
}

impl Runtime {
    /// Traduce un mensaje del canal en una acción (o la aplica él mismo).
    fn translate(&mut self, state: &AppState, msg: Msg) -> Option<Action> {
        match msg {
            Msg::Action(a) => Some(a),
            Msg::Input(ev) => match ev {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    Some(keymap::map_key(state, key))
                }
                Event::Mouse(m) if m.kind != MouseEventKind::Moved => Some(Action::Mouse(m)),
                Event::Paste(text) => Some(Action::Paste(text)),
                Event::Resize(..) => Some(Action::Resize),
                _ => None,
            },
            Msg::Pane(name, pane, ev) => match ev {
                PaneEvent::Exited(status) => {
                    let t = self.tasks.get_mut(&name)?;
                    let program = match pane {
                        Pane::Editor => {
                            t.editor = None;
                            t.nvim = None;
                            state.editor_name.clone()
                        }
                        _ => {
                            t.terminal = None;
                            state.shell_name.clone()
                        }
                    };
                    let how = match &status.signal {
                        Some(sig) => format!("por la señal {sig}"),
                        None => format!("con código {}", status.code),
                    };
                    Some(Action::PaneNote(
                        name,
                        pane,
                        Some(format!("{program} terminó {how}. Enter lo vuelve a abrir.")),
                    ))
                }
                // La salida solo pide redibujar, que ocurre en cada vuelta del bucle.
                PaneEvent::Output | PaneEvent::Title(_) | PaneEvent::Bell => None,
            },
        }
    }

    /// Ajusta los PTY de la tarea activa al tamaño de sus paneles.
    fn resize_active(&mut self, state: &AppState) {
        let (w, h) = state.size;
        if view::tier(w, h) == Tier::TooSmall {
            return;
        }
        let areas = layout::compute(Rect::new(0, 0, w, h), state);
        let size = |r: Option<Rect>| {
            r.map(layout::inner)
                .filter(|i| i.width > 0 && i.height > 0)
                .map(|i| PaneSize::new(i.height, i.width))
        };
        if let Some(s) = size(areas.editor) {
            self.sizes.editor = Some(s);
        }
        if !areas.terminal_minimized {
            if let Some(s) = size(areas.terminal) {
                self.sizes.terminal = Some(s);
            }
        }
        let Some(t) = state
            .active_task()
            .and_then(|t| self.tasks.get_mut(&t.name))
        else {
            return;
        };
        for (pane, want) in [
            (&mut t.editor, self.sizes.editor),
            (&mut t.terminal, self.sizes.terminal),
        ] {
            if let (Some(p), Some(want)) = (pane, want) {
                if p.size() != want {
                    let _ = p.resize(want);
                }
            }
        }
    }

    fn exec(&mut self, effect: Effect, state: &AppState, queue: &mut VecDeque<Action>) {
        let toast = |queue: &mut VecDeque<Action>, text: String, level| {
            queue.push_back(Action::Toast(text, level));
        };
        match effect {
            Effect::Quit => {}
            Effect::Activate(name) => {
                self.ensure_agent(&name, queue);
                for pane in [Pane::Editor, Pane::Terminal] {
                    let running = self.tasks.get(&name).and_then(|t| t.pane(pane)).is_some();
                    let noted = state.tasks.iter().any(|t| {
                        t.name == name
                            && match pane {
                                Pane::Editor => t.editor_note.is_some(),
                                _ => t.terminal_note.is_some(),
                            }
                    });
                    // Un programa que terminó se reabre con Enter, no al cambiar de tarea.
                    if !running && !noted {
                        self.spawn_pane(&name, pane, state, queue);
                    }
                }
                self.refresh_files(&name, queue);
            }
            Effect::Prompt(name, text) => {
                self.ensure_agent(&name, queue);
                let Some(agent) = self.tasks.get(&name).and_then(|t| t.agent.as_ref()) else {
                    return;
                };
                if let Err(e) = agent.prompt(text) {
                    queue.push_back(Action::Agent(name, AgentEvent::TurnFailed(e.to_string())));
                }
            }
            Effect::Cancel(name) => {
                if let Some(a) = self.tasks.get(&name).and_then(|t| t.agent.as_ref()) {
                    let _ = a.cancel();
                }
            }
            Effect::Permission(name, id, option) => {
                if let Some(a) = self.tasks.get(&name).and_then(|t| t.agent.as_ref()) {
                    let choice = option.map_or(PermissionChoice::Cancel, PermissionChoice::Option);
                    let _ = a.respond_permission(id, choice);
                }
            }
            Effect::SetMode(name, mode) => {
                if let Some(a) = self.tasks.get(&name).and_then(|t| t.agent.as_ref()) {
                    let _ = a.set_mode(mode);
                }
            }
            Effect::CreateTask { name, agent, base } => {
                match self.ws.create_task(NewTask {
                    name: name.clone(),
                    base: Some(base),
                    agent: Some(agent),
                }) {
                    Ok(created) => {
                        let mut trt = TaskRt::new(created.task.clone());
                        trt.pending_setup = self.ws.config().setup.clone();
                        self.tasks.insert(name, trt);
                        queue.push_back(Action::TaskCreated(
                            Box::new(ui_task(&created.task)),
                            created.warnings,
                        ));
                    }
                    Err(e) => toast(
                        queue,
                        format!("No se creó la tarea: {e}"),
                        ToastLevel::Error,
                    ),
                }
            }
            Effect::RemoveTask(name) => {
                if let Some(t) = self.tasks.get_mut(&name) {
                    // Primero los procesos: el worktree no se puede borrar con ellos dentro
                    // (en Windows) y el agente no debe seguir escribiendo.
                    t.agent = None;
                    t.editor = None;
                    t.terminal = None;
                }
                let opts = RemoveOptions {
                    force: true,
                    keep_branch: false,
                };
                match self.ws.remove_task(&name, opts) {
                    Ok(_) => {
                        self.tasks.remove(&name);
                        queue.push_back(Action::TaskRemoved(name));
                    }
                    Err(e) => toast(
                        queue,
                        format!("No se borró la tarea: {e}"),
                        ToastLevel::Error,
                    ),
                }
            }
            Effect::OpenFile(name, path) => self.open_file(&name, &path, state, queue),
            Effect::RefreshFiles(name) => self.refresh_files(&name, queue),
            Effect::LoadReview(name) => self.load_review(&name, queue),
            Effect::Commit(name) => {
                let Some(t) = self.tasks.get(&name) else {
                    return;
                };
                let msg = state
                    .tasks
                    .iter()
                    .find(|x| x.name == name)
                    .and_then(|x| x.title.clone())
                    .unwrap_or_else(|| format!("{name}: cambios del agente"));
                match Git::new(&t.task.worktree).commit_all(&msg) {
                    Ok(sha) => toast(
                        queue,
                        format!("Commit {} en {name}", short(&sha)),
                        ToastLevel::Info,
                    ),
                    Err(e) => toast(queue, format!("No se hizo commit: {e}"), ToastLevel::Error),
                }
                self.load_review(&name, queue);
                self.refresh_files(&name, queue);
            }
            Effect::Merge(name) => {
                let main = self.ws.git();
                self.in_background(&name, queue, move |task| merge(&main, &task));
            }
            Effect::PullRequest(name) => {
                let title = state
                    .tasks
                    .iter()
                    .find(|x| x.name == name)
                    .and_then(|x| x.title.clone())
                    .unwrap_or_else(|| name.clone());
                self.in_background(&name, queue, move |task| pull_request(&task, &title));
            }
            Effect::RecordSession(name, info) => {
                if let Some(id) = &info.session_id {
                    if let Some(t) = self.tasks.get_mut(&name) {
                        t.task.session_id = Some(id.clone());
                    }
                }
                if let Err(e) = self.ws.record_session(&name, info) {
                    toast(
                        queue,
                        format!("No se guardó la sesión de {name}: {e}"),
                        ToastLevel::Warning,
                    );
                }
            }
            Effect::PaneInput(name, pane, input) => {
                let Some(p) = self.tasks.get(&name).and_then(|t| t.pane(pane)) else {
                    return;
                };
                let r = match input {
                    PaneInput::Key(k) => {
                        p.scroll_to_bottom();
                        p.send_key(k).map(|_| ())
                    }
                    PaneInput::Mouse(m) => p.send_mouse(m).map(|_| ()),
                    PaneInput::Wheel(m) => p.wheel(m).map(|_| ()),
                    PaneInput::Paste(text) => {
                        p.scroll_to_bottom();
                        p.paste(&text)
                    }
                    PaneInput::Bytes(b) => p.write(&b),
                };
                // Si el programa ya terminó, el aviso llega por su evento `Exited`.
                drop(r);
            }
            Effect::SaveChat(name) => {
                let Some(t) = state.tasks.iter().find(|t| t.name == name) else {
                    return;
                };
                if let Err(e) = save_chat(&self.ws.paths().chat_file(&name), &t.chat) {
                    toast(
                        queue,
                        format!("No se guardó el chat de {name}: {e}"),
                        ToastLevel::Warning,
                    );
                }
            }
            Effect::RestartPane(name, pane) => {
                queue.push_back(Action::PaneNote(name.clone(), pane, None));
                self.spawn_pane(&name, pane, state, queue);
            }
        }
    }

    fn ensure_agent(&mut self, name: &str, queue: &mut VecDeque<Action>) {
        let Some(t) = self.tasks.get_mut(name) else {
            return;
        };
        if t.agent.is_some() {
            return;
        }
        let config = match self.ws.config().agent(Some(&t.task.agent)) {
            Ok((_, c)) => c.clone(),
            Err(e) => {
                queue.push_back(Action::AgentFailed(name.to_string(), e.to_string()));
                return;
            }
        };
        let command = AgentCommand {
            program: config.command.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
        };
        let session = SessionSpec {
            cwd: t.task.worktree.clone(),
            resume: t.task.session_id.clone(),
            mode: config.mode.clone(),
        };
        let tx = self.tx.clone();
        let task_name = name.to_string();
        let sink = move |ev| {
            let _ = tx.send(Msg::Action(Action::Agent(task_name.clone(), ev)));
        };
        match AgentHandle::spawn(&command, session, sink) {
            Ok(h) => {
                t.agent = Some(h);
                queue.push_back(Action::AgentStarting(name.to_string()));
            }
            Err(e) => queue.push_back(Action::AgentFailed(name.to_string(), e.to_string())),
        }
    }

    fn spawn_pane(
        &mut self,
        name: &str,
        pane: Pane,
        state: &AppState,
        queue: &mut VecDeque<Action>,
    ) {
        let config = self.ws.config();
        let editor = config.editor_command();
        let shell = config.shell_command();
        let Some(t) = self.tasks.get_mut(name) else {
            return;
        };
        let cwd = t.task.worktree.clone();
        if !cwd.is_dir() {
            queue.push_back(Action::PaneNote(
                name.to_string(),
                pane,
                Some(format!(
                    "El worktree de la tarea no existe: {}",
                    cwd.display()
                )),
            ));
            return;
        }
        let size = match pane {
            Pane::Editor => self.sizes.editor,
            _ => self.sizes.terminal,
        }
        .unwrap_or(PaneSize::new(24, 80));
        let mut nvim = None;
        let mut kind = EditorKind::Other;
        let command = match pane {
            Pane::Editor => {
                let Some(editor) = editor else {
                    queue.push_back(Action::PaneNote(
                        name.to_string(),
                        pane,
                        Some(
                            "Sin editor: no se encontró nvim, hx, vim, micro, nano ni vi. \
                             Instala uno o define `editor` en la config de qorvi o $EDITOR."
                                .to_string(),
                        ),
                    ));
                    return;
                };
                match editor_command(&editor, &cwd, name) {
                    Ok(spec) => {
                        nvim = spec.remote;
                        kind = spec.kind;
                        spec.command
                    }
                    Err(e) => {
                        queue.push_back(Action::PaneNote(
                            name.to_string(),
                            pane,
                            Some(e.to_string()),
                        ));
                        return;
                    }
                }
            }
            _ => match qorvi_pty::editor::split_command_line(&shell) {
                Ok(parts) if !parts.is_empty() => {
                    PaneCommand::new(&parts[0], &cwd).args(parts[1..].iter().cloned())
                }
                _ => PaneCommand::new(&shell, &cwd),
            },
        };
        let command = command
            .size(size)
            .scrollback(SCROLLBACK)
            .env("QORVI_TASK", name);
        let tx = self.tx.clone();
        let task_name = name.to_string();
        let notify = move |ev| {
            let _ = tx.send(Msg::Pane(task_name.clone(), pane, ev));
        };
        match PtyPane::spawn(command, notify) {
            Ok(p) => {
                if pane == Pane::Terminal {
                    if let Some(setup) = t.pending_setup.take() {
                        let _ = p.write(format!("{setup}\r").as_bytes());
                    }
                    t.terminal = Some(p);
                } else {
                    t.editor = Some(p);
                    t.nvim = nvim;
                    t.editor_kind = kind;
                }
            }
            Err(e) => {
                let program = match pane {
                    Pane::Editor => &state.editor_name,
                    _ => &state.shell_name,
                };
                queue.push_back(Action::PaneNote(
                    name.to_string(),
                    pane,
                    Some(format!("No se pudo abrir {program}: {e}")),
                ));
            }
        }
    }

    fn open_file(
        &mut self,
        name: &str,
        path: &str,
        state: &AppState,
        queue: &mut VecDeque<Action>,
    ) {
        if self.tasks.get(name).is_some_and(|t| t.editor.is_none()) {
            self.spawn_pane(name, Pane::Editor, state, queue);
        }
        let Some(t) = self.tasks.get(name) else {
            return;
        };
        let rel = Path::new(path);
        let result = if let Some(nvim) = &t.nvim {
            nvim.open_file(&t.task.worktree.join(rel), None)
                .map_err(|e| e.to_string())
        } else if let (Some(keys), Some(p)) = (open_file_keys(t.editor_kind, rel, None), &t.editor)
        {
            p.write(&keys).map_err(|e| e.to_string())
        } else {
            Err(format!(
                "{} no se puede controlar desde qorvi: abre {path} a mano",
                state.editor_name
            ))
        };
        match result {
            Ok(()) => queue.push_back(Action::EditorOpened(name.to_string(), path.to_string())),
            Err(e) => queue.push_back(Action::Toast(e, ToastLevel::Warning)),
        }
    }

    fn refresh_files(&self, name: &str, queue: &mut VecDeque<Action>) {
        let Some(t) = self.tasks.get(name) else {
            return;
        };
        match explorer_files(&t.task.worktree) {
            Ok(files) => queue.push_back(Action::FilesLoaded(name.to_string(), files)),
            Err(e) => queue.push_back(Action::Toast(
                format!("No se pudo leer el árbol de {name}: {e}"),
                ToastLevel::Warning,
            )),
        }
    }

    fn load_review(&self, name: &str, queue: &mut VecDeque<Action>) {
        let Some(t) = self.tasks.get(name) else {
            return;
        };
        match review_files(&t.task) {
            Ok(files) => queue.push_back(Action::ReviewLoaded(name.to_string(), files)),
            Err(e) => queue.push_back(Action::Toast(
                format!("No se pudieron leer los cambios: {e}"),
                ToastLevel::Error,
            )),
        }
    }

    /// Ejecuta `op` (merge, push…) en un hilo para no bloquear la interfaz; el
    /// resultado llega como `BusyDone`. `update` ya marcó la operación como en curso.
    fn in_background(
        &self,
        name: &str,
        queue: &mut VecDeque<Action>,
        op: impl FnOnce(CoreTask) -> Result<String, String> + Send + 'static,
    ) {
        let Some(task) = self.tasks.get(name).map(|t| t.task.clone()) else {
            queue.push_back(Action::BusyDone(
                format!("La tarea {name} ya no existe"),
                ToastLevel::Error,
            ));
            return;
        };
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new()
            .name("qorvi-git".into())
            .spawn(move || {
                let done = match op(task) {
                    Ok(msg) => Action::BusyDone(msg, ToastLevel::Info),
                    Err(msg) => Action::BusyDone(msg, ToastLevel::Error),
                };
                let _ = tx.send(Msg::Action(done));
            });
        if let Err(e) = spawned {
            queue.push_back(Action::BusyDone(
                format!("No se pudo lanzar la operación: {e}"),
                ToastLevel::Error,
            ));
        }
    }

    /// Cierra los agentes (esperando a que salgan) y los paneles.
    fn shutdown(&mut self) {
        for (_, t) in self.tasks.drain() {
            if let Some(a) = t.agent {
                a.shutdown();
            }
        }
    }
}

/// Integra la rama de la tarea en su rama base, en el árbol principal (`main`).
fn merge(main: &Git, task: &CoreTask) -> Result<String, String> {
    let name = task.name.as_str();
    if !Git::new(&task.worktree)
        .is_clean()
        .map_err(|e| e.to_string())?
    {
        return Err(format!(
            "{name} tiene cambios sin commitear: haz commit (c) antes del merge"
        ));
    }
    let current = main.current_branch().map_err(|e| e.to_string())?;
    if current.as_deref() != Some(task.base_ref.as_str()) {
        return Err(format!(
            "El repositorio principal está en {} y no en {}: cámbialo antes del merge",
            current.as_deref().unwrap_or("(HEAD separado)"),
            task.base_ref
        ));
    }
    let sha = main
        .merge_branch(&task.branch, MergeStyle::Merge, &format!("Merge: {name}"))
        .map_err(|e| format!("No se hizo el merge: {e}"))?;
    Ok(format!(
        "{name} integrada en {} ({})",
        task.base_ref,
        short(&sha)
    ))
}

/// Sube la rama y abre un PR hacia la rama base con `gh` (o da el que ya está abierto).
fn pull_request(task: &CoreTask, title: &str) -> Result<String, String> {
    let name = task.name.as_str();
    let git = Git::new(&task.worktree);
    if !git.is_clean().map_err(|e| e.to_string())? {
        return Err(format!(
            "{name} tiene cambios sin commitear: haz commit (c) antes del PR"
        ));
    }
    let remote = git
        .remotes()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|r| r == "origin")
        .ok_or("El repositorio no tiene el remoto `origin`")?;
    git.push(&remote, &task.branch, true)
        .map_err(|e| format!("No se pudo hacer push: {e}"))?;
    let gh = Gh::new(&task.worktree);
    if let Ok(Some(pr)) = gh.pr_for_branch(&task.branch) {
        if pr.state == "OPEN" {
            return Ok(format!("Rama subida; PR ya abierto: {}", pr.url));
        }
    }
    let url = gh
        .create_pr(
            &task.base_ref,
            &task.branch,
            title,
            "Creado con qorvi.",
            false,
        )
        .map_err(|e| format!("No se pudo crear el PR: {e}"))?;
    Ok(format!("PR creado: {url}"))
}

/// Mensajes del chat que se guardan por tarea (los más recientes).
const MAX_CHAT_ITEMS: usize = 500;

/// Historial guardado de una tarea; vacío si no hay o no se puede leer. Los permisos
/// que quedaron sin responder ya no tienen a quién responder: se marcan como descartados.
pub fn load_chat(path: &Path) -> Vec<ChatItem> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut chat: Vec<ChatItem> = serde_json::from_str(&text).unwrap_or_default();
    for item in &mut chat {
        if let ChatItem::Permission {
            options,
            resolved,
            request,
            ..
        } = item
        {
            *request = None;
            if resolved.is_none() {
                *resolved = Some(options.len());
            }
        }
    }
    chat
}

/// Guarda el historial (los últimos [`MAX_CHAT_ITEMS`]) con escritura atómica.
pub fn save_chat(path: &Path, chat: &[ChatItem]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("ruta sin directorio"))?;
    std::fs::create_dir_all(dir)?;
    let start = chat.len().saturating_sub(MAX_CHAT_ITEMS);
    let json = serde_json::to_vec(&chat[start..]).map_err(io::Error::other)?;
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Archivos del explorador con su marca de `git status`. Los borrados también salen.
pub fn explorer_files(worktree: &Path) -> qorvi_core::Result<Vec<FileEntry>> {
    let git = Git::new(worktree);
    let mut marks: HashMap<PathBuf, ChangeMark> = HashMap::new();
    for s in git.status()? {
        let state = s.unstaged.as_ref().or(s.staged.as_ref());
        let mark = match state {
            Some(FileState::Untracked) => ChangeMark::Untracked,
            Some(FileState::Added) => ChangeMark::Added,
            Some(FileState::Deleted) => ChangeMark::Deleted,
            Some(_) => ChangeMark::Modified,
            None => continue,
        };
        marks.insert(s.path, mark);
    }
    let mut files: Vec<FileEntry> = git
        .list_files()?
        .into_iter()
        .map(|p| FileEntry {
            mark: marks.remove(&p),
            path: slash(&p),
        })
        .collect();
    files.extend(
        marks
            .into_iter()
            .filter(|(_, m)| *m == ChangeMark::Deleted)
            .map(|(p, m)| FileEntry {
                path: slash(&p),
                mark: Some(m),
            }),
    );
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Cambios de la tarea respecto a su base, con el diff de cada archivo.
pub fn review_files(task: &CoreTask) -> qorvi_core::Result<Vec<ChangedFile>> {
    let git = Git::new(&task.worktree);
    git.changes_since(&task.base_commit)?
        .into_iter()
        .map(|c| {
            let diff = git.file_diff(&task.base_commit, &c.path)?;
            Ok(ChangedFile {
                path: slash(&c.path),
                kind: match c.kind {
                    qorvi_core::ChangeKind::Added => ChangeKind::Added,
                    qorvi_core::ChangeKind::Deleted => ChangeKind::Deleted,
                    _ => ChangeKind::Modified,
                },
                diff: diff.lines().map(str::to_string).collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
    }

    #[test]
    fn archivos_del_explorador_con_marcas() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        git(d, &["init", "-q", "-b", "main"]);
        std::fs::create_dir(d.join("src")).unwrap();
        for f in ["a.txt", "b.txt", "src/c.rs"] {
            std::fs::write(d.join(f), "x\n").unwrap();
        }
        git(d, &["add", "-A"]);
        git(d, &["commit", "-qm", "init"]);
        std::fs::write(d.join("a.txt"), "cambio\n").unwrap();
        std::fs::remove_file(d.join("b.txt")).unwrap();
        std::fs::write(d.join("nuevo.txt"), "n\n").unwrap();

        let files = explorer_files(d).unwrap();
        let get = |p: &str| files.iter().find(|f| f.path == p).map(|f| f.mark);
        assert_eq!(get("a.txt"), Some(Some(ChangeMark::Modified)));
        assert_eq!(get("b.txt"), Some(Some(ChangeMark::Deleted)));
        assert_eq!(get("nuevo.txt"), Some(Some(ChangeMark::Untracked)));
        assert_eq!(get("src/c.rs"), Some(None));
    }

    #[test]
    fn chat_se_guarda_y_se_recupera() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("chats/t.json");
        assert!(load_chat(&path).is_empty());
        let chat = vec![
            ChatItem::User("hola".into()),
            ChatItem::Agent("¿qué tal?".into()),
            ChatItem::Permission {
                title: "ls".into(),
                options: vec!["Sí".into(), "No".into()],
                selected: 0,
                resolved: None,
                request: Some(crate::state::PermissionRef {
                    id: 3,
                    option_ids: vec!["si".into(), "no".into()],
                }),
            },
        ];
        save_chat(&path, &chat).unwrap();
        let back = load_chat(&path);
        assert_eq!(back[..2], chat[..2]);
        // El permiso pendiente vuelve descartado y sin petición.
        assert!(matches!(
            &back[2],
            ChatItem::Permission {
                resolved: Some(2),
                request: None,
                ..
            }
        ));
        // Solo se guardan los últimos MAX_CHAT_ITEMS; un archivo roto da un chat vacío.
        let long: Vec<ChatItem> = (0..MAX_CHAT_ITEMS + 10)
            .map(|i| ChatItem::Notice(i.to_string()))
            .collect();
        save_chat(&path, &long).unwrap();
        let back = load_chat(&path);
        assert_eq!(back.len(), MAX_CHAT_ITEMS);
        assert_eq!(back[0], ChatItem::Notice("10".into()));
        std::fs::write(&path, "{roto").unwrap();
        assert!(load_chat(&path).is_empty());
    }

    #[test]
    fn nombre_de_programa() {
        assert_eq!(program_name("/usr/bin/nvim -u x"), "nvim");
        assert_eq!(program_name(r"C:\x\pwsh.exe"), "pwsh");
        assert_eq!(program_name("zsh"), "zsh");
    }
}
