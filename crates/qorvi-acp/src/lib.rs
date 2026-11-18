//! Cliente del Agent Client Protocol para qorvi.
//!
//! [`AgentHandle::spawn`] lanza un agente ACP en el worktree de una tarea, abre o
//! reanuda la sesión y entrega lo que pasa como [`AgentEvent`]. Los accesos `fs/*` del
//! agente se limitan al worktree ([`fs::contain`]) y los permisos se responden cuando
//! el usuario quiera, sin bloquear la conexión.

mod client;
mod event;
pub mod fs;

pub use client::{AgentCommand, AgentHandle, Error, EventSink, SessionSpec};
pub use event::{
    AgentEvent, Cost, Mode, Modes, PermissionChoice, PermissionId, PermissionKind,
    PermissionOption, PermissionRequest, PlanItem, PlanStatus, SessionReady, StopReason,
    ToolCallInfo, ToolCallPatch, ToolStatus, Usage,
};
