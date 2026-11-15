//! Spike ACP: cliente de línea de comandos para hablar con un agente ACP.
//!
//! Uso:
//!   acp-spike [--cwd DIR] [--load SESSION_ID] -- comando [args...]
//!
//! Cada línea de stdin es un prompt. Cuando hay un permiso pendiente, la siguiente
//! línea es el número de la opción elegida. `/cancel` cancela el turno y `/quit` sale.
//! Las líneas que llegan durante un turno esperan hasta que haga falta una respuesta
//! de permiso o el turno termine, así se pueden pasar guiones por stdin.

use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::schema::{
    CancelNotification, ClientCapabilities, ContentBlock, FileSystemCapabilities,
    InitializeRequest, LoadSessionRequest, NewSessionRequest, PermissionOption, PromptRequest,
    ReadTextFileRequest, ReadTextFileResponse, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionId, SessionNotification,
    SessionUpdate, TextContent, ToolCallContent, WriteTextFileRequest, WriteTextFileResponse,
};
use agent_client_protocol::v1::{ConnectionTo, Responder};
use agent_client_protocol::{Agent, ByteStreams, Client};
use anyhow::{bail, Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

struct Pending {
    options: Vec<PermissionOption>,
    responder: Responder<RequestPermissionResponse>,
}

#[derive(Default)]
struct State {
    turn_active: bool,
    pending: Option<Pending>,
}

type Shared = Arc<Mutex<State>>;

fn t0() -> &'static Instant {
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    T0.get_or_init(Instant::now)
}

macro_rules! log {
    ($($arg:tt)*) => { println!("[{:>6} ms] {}", t0().elapsed().as_millis(), format!($($arg)*)) };
}

#[tokio::main]
async fn main() -> Result<()> {
    t0();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut cwd = std::env::current_dir()?;
    let mut load: Option<String> = None;
    while let Some(a) = args.first().cloned() {
        match a.as_str() {
            "--cwd" => {
                let dir = args.get(1).filter(|d| !d.is_empty() && *d != "--");
                cwd = PathBuf::from(dir.context("--cwd necesita una carpeta (¿variable vacía?)")?);
                args.drain(..2);
            }
            "--load" => {
                load = Some(args.get(1).context("falta SESSION_ID")?.clone());
                args.drain(..2);
            }
            "--" => {
                args.remove(0);
                break;
            }
            _ => break,
        }
    }
    let (program, prog_args) = args.split_first().context("falta el comando del agente")?;
    let cwd = std::fs::canonicalize(&cwd)
        .with_context(|| format!("la carpeta de --cwd no existe: {}", cwd.display()))?;

    // Lanzamos el proceso nosotros (AcpAgent no permite fijar cwd).
    let mut child = Command::new(program)
        .args(prog_args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("no se pudo lanzar {program}"))?;
    let stdin = child.stdin.take().context("stdin")?;
    let stdout = child.stdout.take().context("stdout")?;
    let stderr = child.stderr.take().context("stderr")?;
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log!("[stderr] {line}");
        }
    });

    // Líneas del usuario (stdin del spike) por un canal.
    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });

    let state: Shared = Arc::default();
    let transport = ByteStreams::new(stdin.compat_write(), stdout.compat());
    let fs_root = cwd.clone();
    let fs_root2 = cwd.clone();
    let perm_state = Arc::clone(&state);

    let result = Client
        .v1()
        .name("qorvi-acp-spike")
        .on_receive_notification(
            async move |n: SessionNotification, _cx| {
                print_update(&n.update);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: RequestPermissionRequest, responder, _cx| {
                log!(
                    "PERMISO para «{}»:",
                    req.tool_call.fields.title.clone().unwrap_or_default()
                );
                for (i, o) in req.options.iter().enumerate() {
                    log!("  {} = {} ({:?})", i + 1, o.name, o.kind);
                }
                // No se espera aquí la respuesta: bloquearía el bucle de despacho.
                if let Ok(mut s) = perm_state.lock() {
                    s.pending = Some(Pending {
                        options: req.options,
                        responder,
                    });
                }
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: ReadTextFileRequest, responder, _cx| match contain(&fs_root, &req.path)
            {
                Ok(path) => {
                    log!("fs/read_text_file {} -> permitido", req.path.display());
                    match tokio::fs::read_to_string(&path).await {
                        Ok(content) => responder.respond(ReadTextFileResponse::new(content)),
                        Err(e) => responder.respond_with_error(rpc_error(e)),
                    }
                }
                Err(e) => {
                    log!("fs/read_text_file {} -> RECHAZADO: {e}", req.path.display());
                    responder.respond_with_error(rpc_error(e))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: WriteTextFileRequest, responder, _cx| match contain(
                &fs_root2, &req.path,
            ) {
                Ok(path) => {
                    log!(
                        "fs/write_text_file {} ({} bytes) -> permitido",
                        req.path.display(),
                        req.content.len()
                    );
                    match tokio::fs::write(&path, &req.content).await {
                        Ok(()) => responder.respond(WriteTextFileResponse::new()),
                        Err(e) => responder.respond_with_error(rpc_error(e)),
                    }
                }
                Err(e) => {
                    log!(
                        "fs/write_text_file {} -> RECHAZADO: {e}",
                        req.path.display()
                    );
                    responder.respond_with_error(rpc_error(e))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async |cx: ConnectionTo<Agent>| {
            let init = cx
                .send_request(
                    InitializeRequest::new(ProtocolVersion::V1).client_capabilities(
                        ClientCapabilities::new().fs(FileSystemCapabilities::new()
                            .read_text_file(true)
                            .write_text_file(true)),
                    ),
                )
                .block_task()
                .await?;
            log!(
                "initialize OK: agente={:?} load_session={} auth_methods={}",
                init.agent_info
                    .as_ref()
                    .map(|i| format!("{} {}", i.name, i.version)),
                init.agent_capabilities.load_session,
                init.auth_methods.len()
            );

            let session_id: SessionId = match load {
                Some(id) => {
                    cx.send_request(LoadSessionRequest::new(id.clone(), cwd.clone()))
                        .block_task()
                        .await?;
                    log!("session/load OK: {id}");
                    id.into()
                }
                None => {
                    let s = cx
                        .send_request(NewSessionRequest::new(cwd.clone()))
                        .block_task()
                        .await?;
                    log!("session/new OK: {}", s.session_id);
                    s.session_id
                }
            };

            while let Some(line) = line_rx.recv().await {
                // Un prompt vacío cuesta un turno completo del modelo: nunca se envía.
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                // "!N" responde a un permiso: espera a que haya uno pendiente y se
                // descarta si el turno termina sin pedirlo. El resto de líneas esperan
                // a que no haya turno en curso ("/cancel" va inmediato).
                let is_answer = line.starts_with('!');
                loop {
                    let (pending, active) = state
                        .lock()
                        .map(|s| (s.pending.is_some(), s.turn_active))
                        .unwrap_or((false, false));
                    if line == "/cancel"
                        || (is_answer && (pending || !active))
                        || (!is_answer && !active)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
                let pending = if is_answer {
                    state.lock().ok().and_then(|mut s| s.pending.take())
                } else {
                    None
                };
                if is_answer && pending.is_none() {
                    log!("(sin permiso pendiente: se ignora «{line}»)");
                    continue;
                }
                if let Some(p) = pending {
                    let choice = line[1..]
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| p.options.get(n.wrapping_sub(1)));
                    let outcome = match choice {
                        Some(o) => {
                            log!("-> elegido: {}", o.name);
                            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                                o.option_id.clone(),
                            ))
                        }
                        None => {
                            log!("-> respuesta no válida: se cancela el permiso");
                            RequestPermissionOutcome::Cancelled
                        }
                    };
                    p.responder
                        .respond(RequestPermissionResponse::new(outcome))?;
                    continue;
                }
                match line.as_str() {
                    "/quit" => break,
                    "/cancel" => {
                        log!("-> session/cancel");
                        cx.send_notification(CancelNotification::new(session_id.clone()))?;
                    }
                    prompt => {
                        if let Ok(mut s) = state.lock() {
                            s.turn_active = true;
                        }
                        log!("> {prompt}");
                        let cx2 = cx.clone();
                        let st = Arc::clone(&state);
                        let req = PromptRequest::new(
                            session_id.clone(),
                            vec![ContentBlock::Text(TextContent::new(prompt.to_string()))],
                        );
                        // En una tarea aparte para seguir leyendo líneas (permisos, cancel).
                        cx.spawn(async move {
                            let r = cx2.send_request(req).block_task().await;
                            match &r {
                                Ok(resp) => log!("fin de turno: {:?}", resp.stop_reason),
                                Err(e) => log!("error en el turno: {e}"),
                            }
                            if let Ok(mut s) = st.lock() {
                                s.turn_active = false;
                            }
                            Ok(())
                        })?;
                    }
                }
            }
            // Esperar a que termine el turno en curso antes de salir.
            while state.lock().map(|s| s.turn_active).unwrap_or(false) {
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
            log!("sesión: {session_id}");
            Ok(())
        })
        .await;

    match result {
        Ok(()) => Ok(()),
        Err(e) => bail!("conexión terminada con error: {e}"),
    }
}

/// Error JSON-RPC "invalid params" con el motivo como mensaje.
fn rpc_error(e: impl std::fmt::Display) -> agent_client_protocol::Error {
    agent_client_protocol::Error::new(-32602, e.to_string())
}

/// Resuelve `requested` (ruta absoluta según ACP) y exige que quede dentro de `root`.
/// Resuelve symlinks; para archivos que aún no existen, canonicaliza el padre.
fn contain(root: &Path, requested: &Path) -> Result<PathBuf> {
    if !requested.is_absolute() {
        bail!("la ruta debe ser absoluta");
    }
    if requested.components().any(|c| c == Component::ParentDir) {
        bail!("la ruta contiene '..'");
    }
    let resolved = match std::fs::canonicalize(requested) {
        Ok(p) => p,
        Err(_) => {
            let parent = requested.parent().context("ruta sin directorio padre")?;
            let name = requested
                .file_name()
                .context("ruta sin nombre de archivo")?;
            std::fs::canonicalize(parent)
                .context("el directorio padre no existe")?
                .join(name)
        }
    };
    if !resolved.starts_with(root) {
        bail!("fuera del worktree ({})", resolved.display());
    }
    Ok(resolved)
}

fn print_update(update: &SessionUpdate) {
    match update {
        SessionUpdate::AgentMessageChunk(c) => {
            if let ContentBlock::Text(t) = &c.content {
                log!("agente: {:?}", t.text);
            }
        }
        SessionUpdate::AgentThoughtChunk(_) => log!("(pensando…)"),
        SessionUpdate::ToolCall(tc) => log!(
            "tool_call {} [{:?}/{:?}] «{}»",
            tc.tool_call_id,
            tc.kind,
            tc.status,
            tc.title
        ),
        SessionUpdate::ToolCallUpdate(u) => {
            let diffs = u
                .fields
                .content
                .as_ref()
                .map(|c| {
                    c.iter()
                        .filter(|x| matches!(x, ToolCallContent::Diff(_)))
                        .count()
                })
                .unwrap_or(0);
            log!(
                "tool_call_update {} status={:?} diffs={}",
                u.tool_call_id,
                u.fields.status,
                diffs
            );
        }
        SessionUpdate::Plan(_) => log!("plan actualizado"),
        SessionUpdate::AvailableCommandsUpdate(_) => log!("comandos disponibles actualizados"),
        SessionUpdate::CurrentModeUpdate(m) => log!("modo: {m:?}"),
        SessionUpdate::UsageUpdate(u) => {
            // Solo el que trae coste (el agente manda varios por turno).
            if let Some(c) = &u.cost {
                log!(
                    "uso: contexto {}/{} tokens, coste acumulado {:.4} {}",
                    u.used,
                    u.size,
                    c.amount,
                    c.currency
                );
            }
        }
        SessionUpdate::SessionInfoUpdate(i) => log!("info de sesión: título={:?}", i.title),
        other => {
            let s = format!("{other:?}");
            log!("update: {}", s.chars().take(100).collect::<String>());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::contain;

    #[test]
    fn containment_rules() {
        let tmp = std::env::temp_dir().join(format!("acp-contain-{}", std::process::id()));
        let root = tmp.join("wt");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(tmp.join("secreto.txt"), "x").unwrap();
        let root = std::fs::canonicalize(&root).unwrap();

        assert!(contain(&root, &root.join("src/nuevo.rs")).is_ok());
        assert!(contain(&root, &root.join("../secreto.txt")).is_err());
        assert!(contain(&root, std::path::Path::new("relativo.txt")).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.join("secreto.txt"), root.join("enlace.txt")).unwrap();
            assert!(contain(&root, &root.join("enlace.txt")).is_err());
            std::os::unix::fs::symlink(&tmp, root.join("dir-enlace")).unwrap();
            assert!(contain(&root, &root.join("dir-enlace/nuevo.txt")).is_err());
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
