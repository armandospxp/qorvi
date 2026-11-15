//! Estado de la aplicación: dato puro, sin IO.

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
    Editor,
    Agent,
    Terminal,
}

impl Pane {
    pub fn label(self) -> &'static str {
        match self {
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

#[derive(Debug, Clone, PartialEq)]
pub struct AppState {
    pub tasks: Vec<Task>,
    pub active: usize,
    pub focus: Pane,
    pub zoom: bool,
    pub terminal_minimized: bool,
    pub modal: Modal,
    pub toasts: Vec<Toast>,
    pub last_toast: Option<Toast>,
    pub agents: Vec<String>,
    pub demo: bool,
    pub should_quit: bool,
    pub ascii: bool,
    /// Lo fija el bucle de eventos según el tamaño de la terminal; `update` lo
    /// necesita para saber si Alt+h/l recorren paneles (layout compacto).
    pub compact: bool,
}

impl AppState {
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
