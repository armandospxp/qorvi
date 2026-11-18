//! Eventos que el cliente entrega a la interfaz, sin tipos del SDK.

use std::path::PathBuf;

use agent_client_protocol::schema::MaybeUndefined;
use agent_client_protocol::v1::schema as acp;

/// Identificador de una petición de permiso pendiente (único por conexión).
pub type PermissionId = u64;

/// Algo que pasó en la conexión con el agente.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    /// Conexión y sesión listas; ya se pueden enviar prompts.
    Ready(SessionReady),
    /// Trozo de un mensaje del usuario (al reanudar, el agente repite el historial).
    UserChunk(String),
    /// Trozo del mensaje del agente.
    MessageChunk(String),
    /// Trozo del razonamiento del agente.
    ThoughtChunk(String),
    /// El agente empieza una herramienta.
    ToolCall(ToolCallInfo),
    /// Cambios de una herramienta ya anunciada.
    ToolCallUpdate(ToolCallPatch),
    /// Plan completo del agente (sustituye al anterior).
    Plan(Vec<PlanItem>),
    /// El agente pide permiso. Se responde con [`crate::AgentHandle::respond_permission`].
    PermissionRequest(PermissionRequest),
    /// Un permiso pendiente se resolvió sin que el usuario respondiera (p. ej. al cancelar).
    PermissionCancelled(PermissionId),
    /// Fin del turno.
    TurnEnded(StopReason),
    /// El turno falló (error del agente o de la conexión).
    TurnFailed(String),
    /// Cambió el modo de permisos de la sesión.
    ModeChanged(String),
    /// Uso de contexto y coste acumulado de la sesión.
    Usage(Usage),
    /// Título de la sesión (`None` si el agente lo borra).
    Title(Option<String>),
    /// Se rechazó un acceso `fs/*` fuera del worktree.
    FsDenied { path: PathBuf, reason: String },
    /// Aviso para mostrar al usuario (p. ej. modo inicial inexistente).
    Notice(String),
    /// Línea de stderr del agente (diagnóstico).
    Stderr(String),
    /// La conexión terminó. `error` es `None` si se cerró a petición nuestra.
    Exited {
        code: Option<i32>,
        error: Option<String>,
    },
}

/// Datos de la sesión al conectar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReady {
    pub session_id: String,
    /// `true` si se reanudó una sesión anterior (`session/load`).
    pub resumed: bool,
    /// Nombre y versión del agente, si los da.
    pub agent: Option<String>,
    /// Modos de permisos de la sesión, si el agente los tiene.
    pub modes: Option<Modes>,
}

/// Modos de la sesión (p. ej. «default», «acceptEdits», «plan»).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modes {
    pub current: String,
    pub available: Vec<Mode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mode {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallInfo {
    pub id: String,
    pub title: String,
    /// Tipo en minúsculas: `read`, `edit`, `execute`…
    pub kind: String,
    pub status: ToolStatus,
    /// Archivos que la herramienta muestra como diff.
    pub diffs: Vec<PathBuf>,
    /// Texto que acompaña a la herramienta (salida, explicación).
    pub text: Vec<String>,
}

/// Campos que cambian; `None` = sin cambio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallPatch {
    pub id: String,
    pub title: Option<String>,
    pub kind: Option<String>,
    pub status: Option<ToolStatus>,
    pub diffs: Option<Vec<PathBuf>>,
    pub text: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanItem {
    pub content: String,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub id: PermissionId,
    /// Herramienta que pide permiso.
    pub tool_call_id: String,
    pub title: String,
    pub options: Vec<PermissionOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    pub kind: PermissionKind,
}

/// Respuesta del usuario a un permiso.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionChoice {
    /// Id de una de las [`PermissionOption`].
    Option(String),
    /// Descartar sin elegir.
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    /// Tokens de contexto usados y tamaño de la ventana.
    pub used: u64,
    pub size: u64,
    pub cost: Option<Cost>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cost {
    pub amount: f64,
    pub currency: String,
}

// --- Conversión desde el SDK ---

pub(crate) fn text_of(block: &acp::ContentBlock) -> Option<String> {
    match block {
        acp::ContentBlock::Text(t) => Some(t.text.clone()),
        acp::ContentBlock::ResourceLink(l) => Some(l.uri.clone()),
        _ => None,
    }
}

fn tool_status(s: &acp::ToolCallStatus) -> ToolStatus {
    match s {
        acp::ToolCallStatus::Pending => ToolStatus::Pending,
        acp::ToolCallStatus::InProgress => ToolStatus::InProgress,
        acp::ToolCallStatus::Completed => ToolStatus::Completed,
        acp::ToolCallStatus::Failed => ToolStatus::Failed,
        _ => ToolStatus::InProgress,
    }
}

fn tool_kind(k: &acp::ToolKind) -> String {
    let s = match k {
        acp::ToolKind::Read => "read",
        acp::ToolKind::Edit => "edit",
        acp::ToolKind::Delete => "delete",
        acp::ToolKind::Move => "move",
        acp::ToolKind::Search => "search",
        acp::ToolKind::Execute => "execute",
        acp::ToolKind::Think => "think",
        acp::ToolKind::Fetch => "fetch",
        acp::ToolKind::SwitchMode => "switch_mode",
        _ => "other",
    };
    s.to_string()
}

fn split_content(content: &[acp::ToolCallContent]) -> (Vec<PathBuf>, Vec<String>) {
    let mut diffs = Vec::new();
    let mut text = Vec::new();
    for c in content {
        match c {
            acp::ToolCallContent::Diff(d) => diffs.push(d.path.clone()),
            acp::ToolCallContent::Content(c) => text.extend(text_of(&c.content)),
            _ => {}
        }
    }
    (diffs, text)
}

pub(crate) fn permission_kind(k: &acp::PermissionOptionKind) -> PermissionKind {
    match k {
        acp::PermissionOptionKind::AllowOnce => PermissionKind::AllowOnce,
        acp::PermissionOptionKind::AllowAlways => PermissionKind::AllowAlways,
        acp::PermissionOptionKind::RejectOnce => PermissionKind::RejectOnce,
        acp::PermissionOptionKind::RejectAlways => PermissionKind::RejectAlways,
        _ => PermissionKind::Other,
    }
}

pub(crate) fn stop_reason(r: &acp::StopReason) -> StopReason {
    match r {
        acp::StopReason::EndTurn => StopReason::EndTurn,
        acp::StopReason::MaxTokens => StopReason::MaxTokens,
        acp::StopReason::MaxTurnRequests => StopReason::MaxTurnRequests,
        acp::StopReason::Refusal => StopReason::Refusal,
        acp::StopReason::Cancelled => StopReason::Cancelled,
        _ => StopReason::Other,
    }
}

pub(crate) fn modes(m: &acp::SessionModeState) -> Modes {
    Modes {
        current: m.current_mode_id.to_string(),
        available: m
            .available_modes
            .iter()
            .map(|m| Mode {
                id: m.id.to_string(),
                name: m.name.clone(),
                description: m.description.clone(),
            })
            .collect(),
    }
}

/// Traduce una notificación de sesión; `None` si no interesa a la interfaz.
pub(crate) fn from_update(update: &acp::SessionUpdate) -> Option<AgentEvent> {
    use acp::SessionUpdate as U;
    Some(match update {
        U::UserMessageChunk(c) => AgentEvent::UserChunk(text_of(&c.content)?),
        U::AgentMessageChunk(c) => AgentEvent::MessageChunk(text_of(&c.content)?),
        U::AgentThoughtChunk(c) => AgentEvent::ThoughtChunk(text_of(&c.content)?),
        U::ToolCall(tc) => {
            let (diffs, text) = split_content(&tc.content);
            AgentEvent::ToolCall(ToolCallInfo {
                id: tc.tool_call_id.to_string(),
                title: tc.title.clone(),
                kind: tool_kind(&tc.kind),
                status: tool_status(&tc.status),
                diffs,
                text,
            })
        }
        U::ToolCallUpdate(u) => {
            let f = &u.fields;
            let (diffs, text) = match &f.content {
                Some(c) => {
                    let (d, t) = split_content(c);
                    (Some(d), Some(t))
                }
                None => (None, None),
            };
            AgentEvent::ToolCallUpdate(ToolCallPatch {
                id: u.tool_call_id.to_string(),
                title: f.title.clone(),
                kind: f.kind.as_ref().map(tool_kind),
                status: f.status.as_ref().map(tool_status),
                diffs,
                text,
            })
        }
        U::Plan(p) => AgentEvent::Plan(
            p.entries
                .iter()
                .map(|e| PlanItem {
                    content: e.content.clone(),
                    status: match e.status {
                        acp::PlanEntryStatus::Completed => PlanStatus::Completed,
                        acp::PlanEntryStatus::InProgress => PlanStatus::InProgress,
                        _ => PlanStatus::Pending,
                    },
                })
                .collect(),
        ),
        U::CurrentModeUpdate(m) => AgentEvent::ModeChanged(m.current_mode_id.to_string()),
        U::UsageUpdate(u) => AgentEvent::Usage(Usage {
            used: u.used,
            size: u.size,
            cost: u.cost.as_ref().map(|c| Cost {
                amount: c.amount,
                currency: c.currency.clone(),
            }),
        }),
        U::SessionInfoUpdate(i) => match &i.title {
            MaybeUndefined::Undefined => return None,
            MaybeUndefined::Null => AgentEvent::Title(None),
            MaybeUndefined::Value(t) => AgentEvent::Title(Some(t.clone())),
        },
        _ => return None,
    })
}
