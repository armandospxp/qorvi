use crossterm::event::{KeyEvent, MouseEvent};
use qorvi_acp::AgentEvent;

use crate::state::{AgentStatus, ChangedFile, FileEntry, Pane, Task, ToastLevel};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    FocusLeft,
    FocusRight,
    FocusUp,
    FocusDown,
    SelectTask(usize),
    NewTask,
    OpenReview,
    ToggleZoom,
    ToggleTerminal,
    ToggleExplorer,
    ShowLastToast,
    Quit,
    Help,
    /// `Ctrl+Space`: el siguiente atajo se lee como atajo global.
    PrefixStart,
    /// `Esc` con el prefijo pendiente.
    PrefixCancel,
    /// `Ctrl+Space` dos veces: envía un `Ctrl+Space` literal al panel.
    PrefixLiteral,
    /// Tecla sin atajo tras el prefijo (su nombre, para el aviso).
    PrefixUnknown(String),
    DemoSetStatus(AgentStatus),
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// Texto pegado (bracketed paste de la terminal).
    Paste(String),
    Tick,
    Resize,

    // --- Resultados del runtime (solo en la app real) ---
    /// Evento del agente de una tarea (por nombre).
    Agent(String, AgentEvent),
    /// Se lanzó el agente de la tarea; falta que abra la sesión.
    AgentStarting(String),
    /// No se pudo lanzar el agente.
    AgentFailed(String, String),
    /// Tarea creada (con los avisos de `qorvi new`).
    TaskCreated(Box<Task>, Vec<String>),
    /// Tarea borrada.
    TaskRemoved(String),
    /// Archivos del worktree para el explorador.
    FilesLoaded(String, Vec<FileEntry>),
    /// Cambios de la tarea para la review.
    ReviewLoaded(String, Vec<ChangedFile>),
    /// El programa de un panel arrancó (`None`) o no corre, con el motivo.
    PaneNote(String, Pane, Option<String>),
    /// El editor abrió un archivo.
    EditorOpened(String, String),
    /// Aviso del runtime.
    Toast(String, ToastLevel),
}

/// Entrada para el programa de un panel. Las coordenadas del ratón son relativas al
/// interior del panel.
#[derive(Debug, Clone, PartialEq)]
pub enum PaneInput {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Wheel(MouseEvent),
    Paste(String),
    Bytes(Vec<u8>),
}
