//! Estado de la aplicación: dato puro, sin IO.

use std::collections::BTreeSet;

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
        title: String,
        kind: String,
        status: ToolStatus,
        expanded: bool,
    },
    Permission {
        title: String,
        options: Vec<String>,
        selected: usize,
        resolved: Option<usize>,
    },
    Notice(String),
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

/// Forma de lanzar atajos que muestran la ayuda y las pistas. Las dos funcionan siempre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutStyle {
    /// `Ctrl+Space` y después la tecla.
    Prefix,
    /// `Alt+tecla`.
    Alt,
}

impl ShortcutStyle {
    /// Por defecto, prefijo en macOS (Option no llega como Alt) y Alt en el resto.
    pub fn os_default() -> Self {
        if cfg!(target_os = "macos") {
            ShortcutStyle::Prefix
        } else {
            ShortcutStyle::Alt
        }
    }

    /// Interpreta `QORVI_KEYS` (`prefix` o `alt`); cualquier otro valor no cuenta.
    pub fn from_env_value(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "prefix" => Some(ShortcutStyle::Prefix),
            "alt" => Some(ShortcutStyle::Alt),
            _ => None,
        }
    }

    pub fn other(self) -> Self {
        match self {
            ShortcutStyle::Prefix => ShortcutStyle::Alt,
            ShortcutStyle::Alt => ShortcutStyle::Prefix,
        }
    }
}

/// Texto de un atajo según la forma por defecto: `Ctrl+Space d` o `Alt+d`.
pub fn shortcut_label(style: ShortcutStyle, key: &str) -> String {
    match style {
        ShortcutStyle::Prefix => format!("Ctrl+Space {key}"),
        ShortcutStyle::Alt => format!("Alt+{key}"),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub tasks: Vec<Task>,
    pub active: usize,
    pub focus: Pane,
    pub zoom: bool,
    pub terminal_minimized: bool,
    /// Alternancia del usuario sobre el valor por defecto del tier (Alt+e).
    pub explorer_toggled: bool,
    pub modal: Modal,
    pub toasts: Vec<Toast>,
    pub last_toast: Option<Toast>,
    pub agents: Vec<String>,
    pub demo: bool,
    pub should_quit: bool,
    pub ascii: bool,
    /// Qué forma de atajo muestran la ayuda y las pistas.
    pub shortcut_style: ShortcutStyle,
    /// Se pulsó `Ctrl+Space` y falta la tecla del atajo.
    pub prefix_pending: bool,
    /// Lo fija el bucle de eventos según el tamaño de la terminal; `update` lo
    /// necesita para saber si Alt+h/l recorren paneles (layout compacto).
    pub compact: bool,
    /// Lo fija el bucle de eventos: el explorador es visible por defecto en el tier completo.
    pub explorer_default: bool,
}

impl AppState {
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
