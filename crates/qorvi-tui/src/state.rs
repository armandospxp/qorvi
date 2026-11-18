//! Estado de la aplicación: dato puro, sin IO.

use std::collections::BTreeSet;

use crossterm::event::KeyModifiers;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Disconnected,
    Connecting,
    Idle,
    Working,
    NeedsPermission,
    Done,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Explorer,
    Editor,
    Agent,
    Terminal,
}

impl Pane {
    pub fn label(self) -> &'static str {
        match self {
            Pane::Explorer => "Archivos",
            Pane::Editor => "Editor",
            Pane::Agent => "Agente",
            Pane::Terminal => "Terminal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatItem {
    User(String),
    Agent(String),
    ToolCall {
        /// Id de la herramienta en el agente (vacío en la demo).
        id: String,
        title: String,
        kind: String,
        status: ToolStatus,
        expanded: bool,
    },
    Permission {
        title: String,
        options: Vec<String>,
        selected: usize,
        /// Opción elegida; `Some(options.len())` = descartado sin elegir.
        resolved: Option<usize>,
        /// Petición real del agente (`None` en la demo).
        request: Option<PermissionRef>,
    },
    Notice(String),
    /// Razonamiento del agente (se muestra atenuado y recortado).
    Thought(String),
    /// Plan del agente: cada entrada con su estado. Un plan nuevo sustituye al del turno.
    Plan(Vec<PlanEntry>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanState {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntry {
    pub content: String,
    pub state: PlanState,
}

/// Petición de permiso del agente a la que responde un [`ChatItem::Permission`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRef {
    pub id: u64,
    /// Ids de las opciones, en el mismo orden que `options`.
    pub option_ids: Vec<String>,
}

/// Modo de permisos del agente (id y nombre visible).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMode {
    pub id: String,
    pub name: String,
}

/// Marca de cambio de un archivo en el explorador (como `git status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeMark {
    Modified,
    Added,
    Deleted,
    Untracked,
}

impl ChangeMark {
    pub fn letter(self) -> char {
        match self {
            ChangeMark::Modified => 'M',
            ChangeMark::Added => 'A',
            ChangeMark::Deleted => 'D',
            ChangeMark::Untracked => '?',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Relativa al worktree, con `/` como separador.
    pub path: String,
    pub mark: Option<ChangeMark>,
}

/// Estado de interfaz del explorador de una tarea.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExplorerState {
    /// Rutas de las carpetas expandidas.
    pub expanded: BTreeSet<String>,
    /// Índice dentro de las filas visibles.
    pub selected: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub name: String,
    pub branch: String,
    pub base_branch: String,
    pub agent: String,
    pub status: AgentStatus,
    pub chat: Vec<ChatItem>,
    pub input: String,
    pub editor_lines: Vec<String>,
    pub terminal_lines: Vec<String>,
    pub cost_usd: f64,
    pub files: Vec<FileEntry>,
    pub explorer: ExplorerState,
    /// Archivo abierto en el editor (simulado en la demo).
    pub editor_file: Option<String>,
    /// Líneas del historial del chat por encima del final (0 = pegado al final).
    pub chat_scroll: usize,
    /// Título de la sesión que da el agente.
    pub title: Option<String>,
    /// Modo de permisos actual y los disponibles (vacío si el agente no tiene modos).
    pub mode: Option<String>,
    pub modes: Vec<AgentMode>,
    /// Mensaje en lugar del editor o la terminal cuando su programa no corre
    /// (p. ej. «nvim terminó · Enter lo reinicia»). Solo en la app real.
    pub editor_note: Option<String>,
    pub terminal_note: Option<String>,
}

impl Task {
    pub fn new(name: &str, agent: &str, base_branch: &str) -> Self {
        Task {
            name: name.to_string(),
            branch: name.to_string(), // la rama se llama igual que la tarea (spec)
            base_branch: base_branch.to_string(),
            agent: agent.to_string(),
            status: AgentStatus::Idle,
            chat: Vec::new(),
            input: String::new(),
            editor_lines: vec![
                "  1 // DEMO: archivo simulado".to_string(),
                "~".to_string(),
                "~".to_string(),
            ],
            terminal_lines: vec!["$ ".to_string()],
            cost_usd: 0.0,
            files: Vec::new(),
            explorer: ExplorerState::default(),
            editor_file: None,
            chat_scroll: 0,
            title: None,
            mode: None,
            modes: Vec::new(),
            editor_note: None,
            terminal_note: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    Added,
    Deleted,
    Untracked,
}

impl ChangeKind {
    pub fn label(self) -> &'static str {
        match self {
            ChangeKind::Modified => "modificado",
            ChangeKind::Added => "añadido",
            ChangeKind::Deleted => "borrado",
            ChangeKind::Untracked => "sin seguimiento",
        }
    }

    pub fn letter(self) -> char {
        match self {
            ChangeKind::Modified => 'M',
            ChangeKind::Added => 'A',
            ChangeKind::Deleted => 'D',
            ChangeKind::Untracked => '?',
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChangedFile {
    pub path: String,
    pub kind: ChangeKind,
    pub diff: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    DiscardTask(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    None,
    NewTask {
        name: String,
        agent_idx: usize,
        base: String,
        field: usize,
    },
    Confirm {
        message: String,
        action: ConfirmAction,
    },
    Review {
        files: Vec<ChangedFile>,
        selected: usize,
    },
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub text: String,
    pub level: ToastLevel,
    pub ttl_ticks: u16,
}

/// Ticks (250 ms) que vive un toast: unos 5 segundos.
pub const TOAST_TTL_TICKS: u16 = 20;

/// Forma de lanzar atajos que muestran la ayuda y las pistas. Todas las formas funcionan
/// cuando la terminal envía la tecla (`Alt+tecla` también vale como prefijo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutStyle {
    /// `Cmd`/`Ctrl` directos (`Cmd+B`).
    Direct,
    /// `Ctrl+Space` y después la tecla.
    Prefix,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub tasks: Vec<Task>,
    pub active: usize,
    pub focus: Pane,
    pub zoom: bool,
    pub terminal_minimized: bool,
    /// Alternancia del usuario sobre el valor por defecto del tier (atajo del explorador).
    pub explorer_toggled: bool,
    pub modal: Modal,
    pub toasts: Vec<Toast>,
    pub last_toast: Option<Toast>,
    pub agents: Vec<String>,
    /// Rama base que propone el diálogo de tarea nueva.
    pub default_base: String,
    /// Nombre del editor y de la shell para los títulos de los paneles.
    pub editor_name: String,
    pub shell_name: String,
    pub demo: bool,
    pub should_quit: bool,
    pub ascii: bool,
    /// Qué forma de atajo muestran la ayuda y las pistas.
    pub shortcut_style: ShortcutStyle,
    /// Modificador de los atajos directos: `SUPER` (Cmd) en macOS, `CONTROL` en el resto.
    pub direct_mod: KeyModifiers,
    /// Se pulsó `Ctrl+Space` y falta la tecla del atajo.
    pub prefix_pending: bool,
    /// Ya se avisó de que la terminal no envía Alt (una vez por sesión).
    pub alt_warned: bool,
    /// Operación de git en curso en segundo plano (merge, push + PR), con su texto.
    pub busy: Option<String>,
    /// Fotograma del spinner de `busy` (avanza con cada tick).
    pub spinner: usize,
    /// Se pidió salir con una operación en curso: el siguiente «salir» sale igualmente.
    pub quit_armed: bool,
    /// Lo fija el bucle de eventos según el tamaño de la terminal; `update` lo
    /// necesita para saber si Alt+h/l recorren paneles (layout compacto).
    pub compact: bool,
    /// Lo fija el bucle de eventos: el explorador es visible por defecto en el tier completo.
    pub explorer_default: bool,
    /// Lo fija el bucle de eventos: tamaño de la terminal (columnas, filas). `update`
    /// lo necesita para saber qué hay bajo el ratón.
    pub size: (u16, u16),
}

impl AppState {
    /// Fotograma actual del spinner de las operaciones en segundo plano.
    pub fn spinner_frame(&self) -> &'static str {
        const UNICODE: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        const ASCII: [&str; 4] = ["|", "/", "-", "\\"];
        if self.ascii {
            ASCII[self.spinner % ASCII.len()]
        } else {
            UNICODE[self.spinner % UNICODE.len()]
        }
    }

    /// ¿Se dibuja el explorador como columna? (en compacto es una pestaña).
    pub fn explorer_visible(&self) -> bool {
        !self.compact && (self.explorer_default ^ self.explorer_toggled)
    }

    /// Aplica el tier de la terminal; si el explorador queda oculto, saca de él el foco.
    pub fn set_layout(&mut self, compact: bool, explorer_default: bool) {
        self.compact = compact;
        self.explorer_default = explorer_default;
        if self.focus == Pane::Explorer && !compact && !self.explorer_visible() {
            self.focus = Pane::Editor;
        }
    }

    pub fn active_task(&self) -> Option<&Task> {
        self.tasks.get(self.active)
    }

    pub fn active_task_mut(&mut self) -> Option<&mut Task> {
        self.tasks.get_mut(self.active)
    }

    pub fn push_toast(&mut self, text: impl Into<String>, level: ToastLevel) {
        let toast = Toast {
            text: text.into(),
            level,
            ttl_ticks: TOAST_TTL_TICKS,
        };
        self.last_toast = Some(toast.clone());
        self.toasts.push(toast);
    }
}
