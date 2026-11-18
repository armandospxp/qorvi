//! Conexión con un agente ACP en su propio hilo, con una API síncrona para la interfaz.
//!
//! Cada agente corre en un hilo con un runtime tokio de un solo hilo. La interfaz envía
//! órdenes por un canal ([`AgentHandle`]) y recibe [`AgentEvent`] por un callback que se
//! llama desde ese hilo (debe ser rápido: típicamente reenvía a un canal).
//!
//! Los permisos nunca bloquean: la petición del agente se guarda con un id y se emite un
//! evento; la conexión sigue atendiendo notificaciones hasta que el usuario responde.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::schema::{
    CancelNotification, ClientCapabilities, ContentBlock, FileSystemCapabilities, Implementation,
    InitializeRequest, LoadSessionRequest, NewSessionRequest, PromptRequest, ReadTextFileRequest,
    ReadTextFileResponse, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionId, SessionModeState,
    SessionNotification, SetSessionModeRequest, TextContent, WriteTextFileRequest,
    WriteTextFileResponse,
};
use agent_client_protocol::v1::{ConnectionTo, Responder};
use agent_client_protocol::{Agent, ByteStreams, Client};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::sync::mpsc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::event::{self, AgentEvent, PermissionChoice, PermissionId, SessionReady};
use crate::{fs, proc};

/// Errores al lanzar el agente o al darle órdenes.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no se pudo lanzar el agente `{program}`: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("la carpeta de la tarea no existe: {path}: {source}")]
    Cwd {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("no se pudo crear el hilo del agente: {0}")]
    Runtime(std::io::Error),
    #[error("ya hay un turno en curso")]
    Busy,
    #[error("la conexión con el agente está cerrada")]
    Closed,
}

/// Qué sesión abrir y dónde.
#[derive(Debug, Clone, Default)]
pub struct SessionSpec {
    /// Carpeta de trabajo (el worktree): cwd del agente y raíz de la contención `fs/*`.
    pub cwd: PathBuf,
    /// Sesión a reanudar con `session/load`; si falla o no se admite, se abre una nueva.
    pub resume: Option<String>,
    /// Modo de permisos inicial (id de modo del agente).
    pub mode: Option<String>,
}

/// Programa del agente (se lanza sin shell).
#[derive(Debug, Clone, Default)]
pub struct AgentCommand {
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

/// Receptor de eventos; se llama desde el hilo del agente.
pub type EventSink = Arc<dyn Fn(AgentEvent) + Send + Sync>;

enum Cmd {
    Prompt(String),
    Cancel,
    Permission(PermissionId, PermissionChoice),
    SetMode(String),
    Shutdown,
}

/// Mando de un agente conectado. Al soltarlo se cierra la conexión.
pub struct AgentHandle {
    tx: mpsc::UnboundedSender<Cmd>,
    busy: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AgentHandle {
    /// Lanza `command` en `session.cwd` y abre (o reanuda) la sesión.
    pub fn spawn(
        command: &AgentCommand,
        session: SessionSpec,
        sink: impl Fn(AgentEvent) + Send + Sync + 'static,
    ) -> Result<AgentHandle, Error> {
        let session = canonical_cwd(session)?;
        let rt = runtime()?;
        let mut child = {
            let _guard = rt.enter();
            let mut cmd = tokio::process::Command::new(&command.program);
            cmd.args(&command.args)
                .envs(&command.env)
                .current_dir(&session.cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            proc::prepare(&mut cmd);
            cmd.spawn().map_err(|source| Error::Spawn {
                program: command.program.clone(),
                source,
            })?
        };
        let mut tree = proc::ProcessTree::attach(&child);
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            unreachable!("stdio configurado como piped");
        };
        let sink: EventSink = Arc::new(sink);
        Self::start(rt, sink.clone(), move |cmds, busy| async move {
            let stderr_sink = sink.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    stderr_sink(AgentEvent::Stderr(line));
                }
            });
            let conn = run(stdout, stdin, session, sink, cmds, busy);
            let exited = tokio::select! {
                r = conn => {
                    // stdin ya se cerró: el agente debería salir solo.
                    let status = match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
                        Ok(s) => s.ok(),
                        Err(_) => {
                            let _ = child.kill().await;
                            None
                        }
                    };
                    AgentEvent::Exited {
                        code: status.and_then(|s| s.code()),
                        error: r.err(),
                    }
                }
                status = child.wait() => {
                    let code = status.ok().and_then(|s| s.code());
                    AgentEvent::Exited {
                        code,
                        error: Some(match code {
                            Some(c) => format!("el agente terminó (código {c})"),
                            None => "el agente terminó".to_string(),
                        }),
                    }
                }
            };
            // Lo que quede del árbol (nietos lanzados por el agente) también se cierra.
            tree.kill().await;
            exited
        })
    }

    /// Conecta con un agente sobre flujos ya abiertos (pruebas, agentes en proceso).
    pub fn connect<R, W>(
        reader: R,
        writer: W,
        session: SessionSpec,
        sink: impl Fn(AgentEvent) + Send + Sync + 'static,
    ) -> Result<AgentHandle, Error>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let session = canonical_cwd(session)?;
        let sink: EventSink = Arc::new(sink);
        Self::start(runtime()?, sink.clone(), move |cmds, busy| async move {
            let r = run(reader, writer, session, sink, cmds, busy).await;
            AgentEvent::Exited {
                code: None,
                error: r.err(),
            }
        })
    }

    fn start<F, Fut>(
        rt: tokio::runtime::Runtime,
        sink: EventSink,
        main: F,
    ) -> Result<AgentHandle, Error>
    where
        F: FnOnce(mpsc::UnboundedReceiver<Cmd>, Arc<AtomicBool>) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = AgentEvent>,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        let busy = Arc::new(AtomicBool::new(false));
        let busy2 = Arc::clone(&busy);
        let thread = std::thread::Builder::new()
            .name("qorvi-acp".into())
            .spawn(move || {
                let exited = rt.block_on(main(rx, busy2));
                sink(exited);
            })
            .map_err(Error::Runtime)?;
        Ok(AgentHandle {
            tx,
            busy,
            thread: Some(thread),
        })
    }

    /// Envía un prompt. Falla con [`Error::Busy`] si hay un turno en curso.
    pub fn prompt(&self, text: impl Into<String>) -> Result<(), Error> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(Error::Busy);
        }
        self.send(Cmd::Prompt(text.into())).inspect_err(|_| {
            self.busy.store(false, Ordering::SeqCst);
        })
    }

    /// `true` mientras hay un turno en curso.
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// Cancela el turno en curso; los permisos pendientes se descartan.
    pub fn cancel(&self) -> Result<(), Error> {
        self.send(Cmd::Cancel)
    }

    /// Responde a una petición de permiso.
    pub fn respond_permission(
        &self,
        id: PermissionId,
        choice: PermissionChoice,
    ) -> Result<(), Error> {
        self.send(Cmd::Permission(id, choice))
    }

    /// Cambia el modo de permisos de la sesión.
    pub fn set_mode(&self, mode: impl Into<String>) -> Result<(), Error> {
        self.send(Cmd::SetMode(mode.into()))
    }

    /// Cierra la conexión y espera a que el hilo termine (y el agente salga).
    pub fn shutdown(mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    fn send(&self, cmd: Cmd) -> Result<(), Error> {
        self.tx.send(cmd).map_err(|_| Error::Closed)
    }
}

impl Drop for AgentHandle {
    fn drop(&mut self) {
        // Sin esperar al hilo: cierra la conexión y mata al agente si no sale.
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Error::Runtime)
}

fn canonical_cwd(mut s: SessionSpec) -> Result<SessionSpec, Error> {
    s.cwd = std::fs::canonicalize(&s.cwd).map_err(|source| Error::Cwd {
        path: s.cwd.clone(),
        source,
    })?;
    Ok(s)
}

/// Permisos pendientes de respuesta.
#[derive(Default)]
struct Pending {
    next_id: PermissionId,
    map: HashMap<PermissionId, Responder<RequestPermissionResponse>>,
}

type Shared = Arc<Mutex<Pending>>;

fn rpc_error(e: impl std::fmt::Display) -> agent_client_protocol::Error {
    agent_client_protocol::Error::new(-32602, e.to_string())
}

/// Conexión completa: handlers, sesión y bucle de órdenes. Devuelve el motivo si
/// termina por un error.
async fn run<R, W>(
    reader: R,
    writer: W,
    session: SessionSpec,
    sink: EventSink,
    cmds: mpsc::UnboundedReceiver<Cmd>,
    busy: Arc<AtomicBool>,
) -> Result<(), String>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let transport = ByteStreams::new(writer.compat_write(), reader.compat());
    let pending: Shared = Arc::default();
    let root = session.cwd.clone();
    let (s1, s2, s3, s4) = (sink.clone(), sink.clone(), sink.clone(), sink.clone());
    let (root1, root2) = (root.clone(), root.clone());
    let pending1 = Arc::clone(&pending);

    Client
        .v1()
        .name("qorvi")
        .on_receive_notification(
            async move |n: SessionNotification, _cx| {
                if let Some(ev) = event::from_update(&n.update) {
                    s1(ev);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: RequestPermissionRequest, responder, _cx| {
                let ev = {
                    let Ok(mut p) = pending1.lock() else {
                        return responder
                            .respond_with_error(agent_client_protocol::Error::internal_error());
                    };
                    p.next_id += 1;
                    let id = p.next_id;
                    p.map.insert(id, responder);
                    event::PermissionRequest {
                        id,
                        tool_call_id: req.tool_call.tool_call_id.to_string(),
                        title: req.tool_call.fields.title.clone().unwrap_or_default(),
                        options: req
                            .options
                            .iter()
                            .map(|o| event::PermissionOption {
                                id: o.option_id.to_string(),
                                name: o.name.clone(),
                                kind: event::permission_kind(&o.kind),
                            })
                            .collect(),
                    }
                };
                s2(AgentEvent::PermissionRequest(ev));
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: ReadTextFileRequest, responder, _cx| match fs::contain(
                &root1, &req.path,
            ) {
                Ok(path) => match fs::read_text(&path, req.line, req.limit).await {
                    Ok(content) => responder.respond(ReadTextFileResponse::new(content)),
                    Err(e) => responder.respond_with_error(rpc_error(e)),
                },
                Err(e) => {
                    s3(AgentEvent::FsDenied {
                        path: req.path.clone(),
                        reason: e.to_string(),
                    });
                    responder.respond_with_error(rpc_error(e))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: WriteTextFileRequest, responder, _cx| match fs::contain(
                &root2, &req.path,
            ) {
                Ok(path) => match fs::write_text(&path, &req.content).await {
                    Ok(()) => responder.respond(WriteTextFileResponse::new()),
                    Err(e) => responder.respond_with_error(rpc_error(e)),
                },
                Err(e) => {
                    s4(AgentEvent::FsDenied {
                        path: req.path.clone(),
                        reason: e.to_string(),
                    });
                    responder.respond_with_error(rpc_error(e))
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            main_loop(cx, session, sink, cmds, busy, pending).await
        })
        .await
        .map_err(|e| e.to_string())
}

async fn main_loop(
    cx: ConnectionTo<Agent>,
    session: SessionSpec,
    sink: EventSink,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
    busy: Arc<AtomicBool>,
    pending: Shared,
) -> Result<(), agent_client_protocol::Error> {
    let init = cx
        .send_request(
            InitializeRequest::new(ProtocolVersion::V1)
                .client_capabilities(
                    ClientCapabilities::new().fs(FileSystemCapabilities::new()
                        .read_text_file(true)
                        .write_text_file(true)),
                )
                .client_info(Implementation::new("qorvi", env!("CARGO_PKG_VERSION"))),
        )
        .block_task()
        .await?;
    let agent = init
        .agent_info
        .as_ref()
        .map(|i| format!("{} {}", i.name, i.version));

    let (session_id, resumed, modes) =
        open_session(&cx, &session, init.agent_capabilities.load_session, &sink).await?;
    let mut modes = modes.as_ref().map(event::modes);
    sink(AgentEvent::Ready(SessionReady {
        session_id: session_id.to_string(),
        resumed,
        agent,
        modes: modes.clone(),
    }));

    if let Some(wanted) = &session.mode {
        match &mut modes {
            None => sink(AgentEvent::Notice(format!(
                "el agente no tiene modos de permisos; se ignora mode = \"{wanted}\""
            ))),
            Some(m) if !m.available.iter().any(|a| &a.id == wanted) => {
                let ids: Vec<&str> = m.available.iter().map(|a| a.id.as_str()).collect();
                sink(AgentEvent::Notice(format!(
                    "el modo \"{wanted}\" no existe en este agente (disponibles: {})",
                    ids.join(", ")
                )));
            }
            Some(m) if &m.current == wanted => {}
            Some(_) => set_mode(&cx, &session_id, wanted, &sink).await,
        }
    }

    while let Some(cmd) = cmds.recv().await {
        match cmd {
            Cmd::Prompt(text) => {
                let req = PromptRequest::new(
                    session_id.clone(),
                    vec![ContentBlock::Text(TextContent::new(text))],
                );
                let (cx2, sink2, busy2) = (cx.clone(), sink.clone(), Arc::clone(&busy));
                cx.spawn(async move {
                    let r = cx2.send_request(req).block_task().await;
                    busy2.store(false, Ordering::SeqCst);
                    sink2(match r {
                        Ok(resp) => AgentEvent::TurnEnded(event::stop_reason(&resp.stop_reason)),
                        Err(e) => AgentEvent::TurnFailed(e.to_string()),
                    });
                    Ok(())
                })?;
            }
            Cmd::Cancel => {
                cx.send_notification(CancelNotification::new(session_id.clone()))?;
                // ACP: tras cancelar, el cliente responde `cancelled` a los permisos pendientes.
                let drained: Vec<_> = pending
                    .lock()
                    .map(|mut p| p.map.drain().collect())
                    .unwrap_or_default();
                for (id, responder) in drained {
                    let _ = responder.respond(RequestPermissionResponse::new(
                        RequestPermissionOutcome::Cancelled,
                    ));
                    sink(AgentEvent::PermissionCancelled(id));
                }
            }
            Cmd::Permission(id, choice) => {
                let responder = pending.lock().ok().and_then(|mut p| p.map.remove(&id));
                if let Some(responder) = responder {
                    let outcome = match choice {
                        PermissionChoice::Option(o) => {
                            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(o))
                        }
                        PermissionChoice::Cancel => RequestPermissionOutcome::Cancelled,
                    };
                    responder.respond(RequestPermissionResponse::new(outcome))?;
                }
            }
            Cmd::SetMode(mode) => {
                let (cx2, sink2, sid) = (cx.clone(), sink.clone(), session_id.clone());
                cx.spawn(async move {
                    set_mode(&cx2, &sid, &mode, &sink2).await;
                    Ok(())
                })?;
            }
            Cmd::Shutdown => break,
        }
    }
    Ok(())
}

/// Reanuda `session.resume` si se puede; si no, abre una sesión nueva.
async fn open_session(
    cx: &ConnectionTo<Agent>,
    session: &SessionSpec,
    can_load: bool,
    sink: &EventSink,
) -> Result<(SessionId, bool, Option<SessionModeState>), agent_client_protocol::Error> {
    if let Some(id) = &session.resume {
        if can_load {
            match cx
                .send_request(LoadSessionRequest::new(id.clone(), session.cwd.clone()))
                .block_task()
                .await
            {
                Ok(r) => return Ok((id.clone().into(), true, r.modes)),
                Err(e) => sink(AgentEvent::Notice(format!(
                    "no se pudo reanudar la sesión {id} ({}); se abre una nueva",
                    e.message
                ))),
            }
        } else {
            sink(AgentEvent::Notice(
                "el agente no permite reanudar sesiones; se abre una nueva".to_string(),
            ));
        }
    }
    let r = cx
        .send_request(NewSessionRequest::new(session.cwd.clone()))
        .block_task()
        .await?;
    Ok((r.session_id, false, r.modes))
}

async fn set_mode(cx: &ConnectionTo<Agent>, sid: &SessionId, mode: &str, sink: &EventSink) {
    match cx
        .send_request(SetSessionModeRequest::new(sid.clone(), mode.to_string()))
        .block_task()
        .await
    {
        Ok(_) => sink(AgentEvent::ModeChanged(mode.to_string())),
        Err(e) => sink(AgentEvent::Notice(format!(
            "no se pudo cambiar al modo \"{mode}\": {}",
            e.message
        ))),
    }
}
