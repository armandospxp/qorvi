use crossterm::event::{KeyEvent, MouseEvent};

use crate::state::AgentStatus;

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
    /// `Ctrl+Space` dos veces: en la app real envía un `Ctrl+Space` literal al panel.
    PrefixLiteral,
    /// Tecla sin atajo tras el prefijo (su nombre, para el aviso).
    PrefixUnknown(String),
    DemoSetStatus(AgentStatus),
    Key(KeyEvent),
    Mouse(MouseEvent),
    Tick,
    Resize,
}
