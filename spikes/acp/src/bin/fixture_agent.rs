//! Agente ACP de prueba con un guion fijo, para probar clientes sin un LLM.
//!
//! Ante cualquier prompt: 3 trozos de texto → tool call que pide permiso → si se
//! aprueba, escribe `nota.txt` en el cwd de la sesión e intenta escribir
//! `../fuera.txt` (el cliente debe rechazarlo) → `end_turn`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::v1::schema::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, InitializeRequest,
    InitializeResponse, NewSessionRequest, NewSessionResponse, PermissionOption,
    PermissionOptionKind, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionNotification, SessionUpdate, StopReason, TextContent,
    ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind, WriteTextFileRequest,
};
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::{Agent, Client, Result, Stdio};

#[tokio::main]
async fn main() -> Result<()> {
    let cwd: Arc<Mutex<PathBuf>> = Arc::default();
    let cwd2 = Arc::clone(&cwd);
    let cancelled = Arc::new(Mutex::new(false));
    let cancelled2 = Arc::clone(&cancelled);

    Agent
        .v1()
        .name("qorvi-fixture-agent")
        .on_receive_request(
            async move |init: InitializeRequest, responder, _cx| {
                eprintln!("fixture: initialize (fs write = {})", init.client_capabilities.fs.write_text_file);
                responder.respond(
                    InitializeResponse::new(init.protocol_version)
                        .agent_capabilities(AgentCapabilities::new()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: NewSessionRequest, responder, _cx| {
                if let Ok(mut c) = cwd.lock() {
                    *c = req.cwd;
                }
                responder.respond(NewSessionResponse::new("sesion-fixture-1"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |_n: CancelNotification, _cx| {
                if let Ok(mut c) = cancelled2.lock() {
                    *c = true;
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: PromptRequest, responder, cx: ConnectionTo<Client>| {
                let sid = req.session_id.clone();
                let dir = cwd2.lock().map(|c| c.clone()).unwrap_or_default();
                let cancelled = Arc::clone(&cancelled);
                if let Ok(mut c) = cancelled.lock() {
                    *c = false;
                }
                let cx2 = cx.clone();
                // El guion corre fuera del bucle de despacho: espera respuestas del cliente.
                cx.spawn(async move {
                    let say = |t: &str| {
                        cx2.send_notification(SessionNotification::new(
                            sid.clone(),
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                                TextContent::new(t.to_string()),
                            ))),
                        ))
                    };
                    for t in ["Hola, ", "soy el agente ", "de prueba."] {
                        say(t)?;
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    cx2.send_notification(SessionNotification::new(
                        sid.clone(),
                        SessionUpdate::ToolCall(
                            ToolCall::new("tc-1", "Escribir nota.txt")
                                .kind(ToolKind::Edit)
                                .status(ToolCallStatus::Pending),
                        ),
                    ))?;
                    let perm = cx2
                        .send_request(RequestPermissionRequest::new(
                            sid.clone(),
                            ToolCallUpdate::new("tc-1", ToolCallUpdateFields::new().title("Escribir nota.txt")),
                            vec![
                                PermissionOption::new("permitir", "Permitir una vez", PermissionOptionKind::AllowOnce),
                                PermissionOption::new("rechazar", "Rechazar", PermissionOptionKind::RejectOnce),
                            ],
                        ))
                        .block_task()
                        .await?;
                    let approved = matches!(
                        &perm.outcome,
                        RequestPermissionOutcome::Selected(s) if s.option_id.to_string() == "permitir"
                    );
                    if *cancelled.lock().map_err(|_| agent_client_protocol::Error::internal_error())? {
                        return responder.respond(PromptResponse::new(StopReason::Cancelled));
                    }
                    if approved {
                        let ok = cx2
                            .send_request(WriteTextFileRequest::new(sid.clone(), dir.join("nota.txt"), "escrito por el fixture\n"))
                            .block_task()
                            .await;
                        say(&format!("nota.txt: {}", if ok.is_ok() { "escrito" } else { "error" }))?;
                        let fuera = cx2
                            .send_request(WriteTextFileRequest::new(sid.clone(), dir.join("../fuera.txt"), "no debería existir\n"))
                            .block_task()
                            .await;
                        say(&format!(" | ../fuera.txt: {}", match fuera { Ok(_) => "ESCRITO (mal)".to_string(), Err(e) => format!("rechazado ({})", e.message) }))?;
                    } else {
                        say("permiso denegado, no escribo nada")?;
                    }
                    cx2.send_notification(SessionNotification::new(
                        sid.clone(),
                        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                            "tc-1",
                            ToolCallUpdateFields::new().status(ToolCallStatus::Completed),
                        )),
                    ))?;
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}
