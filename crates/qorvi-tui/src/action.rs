use crossterm::event::KeyEvent;

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
    DemoSetStatus(AgentStatus),
    Key(KeyEvent),
    Tick,
    Resize,
}
