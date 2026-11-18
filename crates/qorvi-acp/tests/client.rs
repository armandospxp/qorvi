//! Pruebas del cliente contra un agente ACP de guion, en proceso (sin LLM).

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::v1::schema::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, Cost, Implementation,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse,
    NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind, PromptRequest,
    PromptResponse, ReadTextFileRequest, RequestPermissionOutcome, RequestPermissionRequest,
    SessionInfoUpdate, SessionMode, SessionModeState, SessionNotification, SessionUpdate,
    SetSessionModeRequest, SetSessionModeResponse, StopReason as AcpStop, TextContent, ToolCall,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind, UsageUpdate,
    WriteTextFileRequest,
};
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::{Agent, ByteStreams, Client};
use qorvi_acp::{
    AgentEvent, AgentHandle, PermissionChoice, PermissionKind, SessionSpec, StopReason, ToolStatus,
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

const TIMEOUT: Duration = Duration::from_secs(10);

/// Agente de guion. Según el prompt:
/// - `hola`: texto, uso con coste y título → `end_turn`.
/// - `escribir`: tool call + permiso; si se aprueba escribe `nota.txt` dentro y
///   `<padre>/fuera.txt` fuera (debe rechazarse) y cuenta el resultado.
/// - `leer`: lee las líneas 2–3 de `datos.txt` y las repite.
/// - `esperar`: pide permiso y espera; si llega `session/cancel`, termina con `cancelled`.
fn fixture_agent(stream: tokio::io::DuplexStream) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let (r, w) = tokio::io::split(stream);
            let transport = ByteStreams::new(w.compat_write(), r.compat());
            let cwd: Arc<Mutex<PathBuf>> = Arc::default();
            let cwd_new = Arc::clone(&cwd);
            let cwd_load = Arc::clone(&cwd);
            let cancelled = Arc::new(Mutex::new(false));
            let cancelled2 = Arc::clone(&cancelled);
            let modes = || {
                SessionModeState::new(
                    "default",
                    vec![
                        SessionMode::new("default", "Preguntar"),
                        SessionMode::new("plan", "Planificar"),
                    ],
                )
            };
            let _ = Agent
                .v1()
                .name("fixture")
                .on_receive_request(
                    async move |init: InitializeRequest, responder, _cx| {
                        responder.respond(
                            InitializeResponse::new(init.protocol_version)
                                .agent_capabilities(AgentCapabilities::new().load_session(true))
                                .agent_info(Implementation::new("fixture", "1.0")),
                        )
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |req: NewSessionRequest, responder, _cx| {
                        *cwd_new.lock().unwrap() = req.cwd;
                        responder.respond(NewSessionResponse::new("s-nueva").modes(modes()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |req: LoadSessionRequest, responder, cx: ConnectionTo<Client>| {
                        if req.session_id.to_string() != "s-vieja" {
                            return responder.respond_with_error(
                                agent_client_protocol::Error::new(-32002, "sesión desconocida"),
                            );
                        }
                        *cwd_load.lock().unwrap() = req.cwd;
                        cx.send_notification(SessionNotification::new(
                            req.session_id.clone(),
                            SessionUpdate::UserMessageChunk(ContentChunk::new(text("hola de antes"))),
                        ))?;
                        responder.respond(LoadSessionResponse::new().modes(modes()))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_req: SetSessionModeRequest, responder, _cx| {
                        responder.respond(SetSessionModeResponse::new())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_notification(
                    async move |_n: CancelNotification, _cx| {
                        *cancelled2.lock().unwrap() = true;
                        Ok(())
                    },
                    agent_client_protocol::on_receive_notification!(),
                )
                .on_receive_request(
                    async move |req: PromptRequest, responder, cx: ConnectionTo<Client>| {
                        let prompt = match req.prompt.first() {
                            Some(ContentBlock::Text(t)) => t.text.clone(),
                            _ => String::new(),
                        };
                        let sid = req.session_id.clone();
                        let dir = cwd.lock().unwrap().clone();
                        let cancelled = Arc::clone(&cancelled);
                        *cancelled.lock().unwrap() = false;
                        let cx2 = cx.clone();
                        cx.spawn(async move {
                            let say = |t: &str| {
                                cx2.send_notification(SessionNotification::new(
                                    sid.clone(),
                                    SessionUpdate::AgentMessageChunk(ContentChunk::new(text(t))),
                                ))
                            };
                            let ask = || {
                                cx2.send_request(RequestPermissionRequest::new(
                                    sid.clone(),
                                    ToolCallUpdate::new(
                                        "tc-1",
                                        ToolCallUpdateFields::new().title("Escribir nota.txt"),
                                    ),
                                    vec![
                                        PermissionOption::new(
                                            "si",
                                            "Permitir",
                                            PermissionOptionKind::AllowOnce,
                                        ),
                                        PermissionOption::new(
                                            "no",
                                            "Rechazar",
                                            PermissionOptionKind::RejectOnce,
                                        ),
                                    ],
                                ))
                            };
                            let stop = match prompt.as_str() {
                                "hola" => {
                                    say("Hola, ")?;
                                    say("mundo")?;
                                    cx2.send_notification(SessionNotification::new(
                                        sid.clone(),
                                        SessionUpdate::UsageUpdate(
                                            UsageUpdate::new(1200, 200000)
                                                .cost(Cost::new(0.0125, "USD")),
                                        ),
                                    ))?;
                                    cx2.send_notification(SessionNotification::new(
                                        sid.clone(),
                                        SessionUpdate::SessionInfoUpdate(
                                            SessionInfoUpdate::new().title("Saludo"),
                                        ),
                                    ))?;
                                    AcpStop::EndTurn
                                }
                                "escribir" => {
                                    cx2.send_notification(SessionNotification::new(
                                        sid.clone(),
                                        SessionUpdate::ToolCall(
                                            ToolCall::new("tc-1", "Escribir nota.txt")
                                                .kind(ToolKind::Edit)
                                                .status(ToolCallStatus::Pending),
                                        ),
                                    ))?;
                                    let perm = ask().block_task().await?;
                                    let ok = matches!(&perm.outcome,
                                        RequestPermissionOutcome::Selected(s) if s.option_id.to_string() == "si");
                                    if ok {
                                        let dentro = cx2
                                            .send_request(WriteTextFileRequest::new(
                                                sid.clone(),
                                                dir.join("sub/nota.txt"),
                                                "dentro\n",
                                            ))
                                            .block_task()
                                            .await
                                            .is_ok();
                                        let fuera = cx2
                                            .send_request(WriteTextFileRequest::new(
                                                sid.clone(),
                                                dir.parent().unwrap().join("fuera.txt"),
                                                "fuera\n",
                                            ))
                                            .block_task()
                                            .await
                                            .is_ok();
                                        say(&format!("dentro={dentro} fuera={fuera}"))?;
                                    } else {
                                        say("rechazado")?;
                                    }
                                    cx2.send_notification(SessionNotification::new(
                                        sid.clone(),
                                        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                                            "tc-1",
                                            ToolCallUpdateFields::new()
                                                .status(ToolCallStatus::Completed),
                                        )),
                                    ))?;
                                    AcpStop::EndTurn
                                }
                                "leer" => {
                                    let r = cx2
                                        .send_request(
                                            ReadTextFileRequest::new(sid.clone(), dir.join("datos.txt"))
                                                .line(2)
                                                .limit(2),
                                        )
                                        .block_task()
                                        .await?;
                                    say(&r.content)?;
                                    AcpStop::EndTurn
                                }
                                "esperar" => {
                                    let perm = ask().block_task().await?;
                                    let fue_cancelado =
                                        matches!(perm.outcome, RequestPermissionOutcome::Cancelled);
                                    if *cancelled.lock().unwrap() && fue_cancelado {
                                        AcpStop::Cancelled
                                    } else {
                                        AcpStop::EndTurn
                                    }
                                }
                                _ => AcpStop::Refusal,
                            };
                            responder.respond(PromptResponse::new(stop))
                        })?;
                        Ok(())
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .connect_to(transport)
                .await;
        });
    });
}

fn text(t: &str) -> ContentBlock {
    ContentBlock::Text(TextContent::new(t.to_string()))
}

struct Harness {
    handle: AgentHandle,
    rx: mpsc::Receiver<AgentEvent>,
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

fn connect(resume: Option<&str>, mode: Option<&str>) -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("wt");
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let (client_end, agent_end) = tokio::io::duplex(64 * 1024);
    fixture_agent(agent_end);
    let (r, w) = tokio::io::split(client_end);
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let handle = AgentHandle::connect(
        r,
        w,
        SessionSpec {
            cwd: root.clone(),
            resume: resume.map(str::to_string),
            mode: mode.map(str::to_string),
        },
        move |ev| {
            let _ = tx.lock().unwrap().send(ev);
        },
    )
    .unwrap();
    Harness {
        handle,
        rx,
        _tmp: tmp,
        root,
    }
}

impl Harness {
    /// Recoge eventos hasta que uno cumpla `pred`; devuelve todos (incluido ese).
    fn until(&self, pred: impl Fn(&AgentEvent) -> bool) -> Vec<AgentEvent> {
        let mut seen = Vec::new();
        loop {
            match self.rx.recv_timeout(TIMEOUT) {
                Ok(ev) => {
                    let done = pred(&ev);
                    seen.push(ev);
                    if done {
                        return seen;
                    }
                }
                Err(_) => panic!("tiempo agotado; eventos: {seen:#?}"),
            }
        }
    }

    fn ready(&self) -> qorvi_acp::SessionReady {
        let evs = self.until(|e| matches!(e, AgentEvent::Ready(_)));
        match evs.into_iter().last() {
            Some(AgentEvent::Ready(r)) => r,
            _ => unreachable!(),
        }
    }

    fn turn(&self) -> Vec<AgentEvent> {
        self.until(|e| matches!(e, AgentEvent::TurnEnded(_) | AgentEvent::TurnFailed(_)))
    }
}

fn agent_text(evs: &[AgentEvent]) -> String {
    evs.iter()
        .filter_map(|e| match e {
            AgentEvent::MessageChunk(t) => Some(t.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn sesion_nueva_turno_uso_y_titulo() {
    let h = connect(None, None);
    let ready = h.ready();
    assert_eq!(ready.session_id, "s-nueva");
    assert!(!ready.resumed);
    assert_eq!(ready.agent.as_deref(), Some("fixture 1.0"));
    let modes = ready.modes.unwrap();
    assert_eq!(modes.current, "default");
    assert_eq!(modes.available.len(), 2);

    h.handle.prompt("hola").unwrap();
    assert!(h.handle.is_busy());
    assert!(matches!(
        h.handle.prompt("otra"),
        Err(qorvi_acp::Error::Busy)
    ));
    let evs = h.turn();
    assert_eq!(agent_text(&evs), "Hola, mundo");
    assert!(evs.iter().any(|e| matches!(e, AgentEvent::Usage(u)
        if u.used == 1200 && u.cost.as_ref().is_some_and(|c| c.amount == 0.0125 && c.currency == "USD"))));
    assert!(evs.contains(&AgentEvent::Title(Some("Saludo".into()))));
    assert_eq!(
        evs.last(),
        Some(&AgentEvent::TurnEnded(StopReason::EndTurn))
    );
    assert!(!h.handle.is_busy());
    h.handle.shutdown();
}

#[test]
fn permiso_aprobado_y_contencion_fs() {
    let h = connect(None, None);
    h.ready();
    h.handle.prompt("escribir").unwrap();
    let evs = h.until(|e| matches!(e, AgentEvent::PermissionRequest(_)));
    assert!(evs.iter().any(|e| matches!(e, AgentEvent::ToolCall(t)
        if t.id == "tc-1" && t.kind == "edit" && t.status == ToolStatus::Pending)));
    let Some(AgentEvent::PermissionRequest(req)) = evs.last() else {
        unreachable!()
    };
    assert_eq!(req.title, "Escribir nota.txt");
    assert_eq!(req.options[0].kind, PermissionKind::AllowOnce);

    h.handle
        .respond_permission(req.id, PermissionChoice::Option("si".into()))
        .unwrap();
    let evs = h.turn();
    assert_eq!(agent_text(&evs), "dentro=true fuera=false");
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::FsDenied { path, .. } if path.ends_with("fuera.txt"))));
    assert!(evs.iter().any(|e| matches!(e, AgentEvent::ToolCallUpdate(p)
        if p.status == Some(ToolStatus::Completed))));
    assert_eq!(
        std::fs::read_to_string(h.root.join("sub/nota.txt")).unwrap(),
        "dentro\n"
    );
    assert!(!h.root.parent().unwrap().join("fuera.txt").exists());
}

#[test]
fn permiso_rechazado() {
    let h = connect(None, None);
    h.ready();
    h.handle.prompt("escribir").unwrap();
    let evs = h.until(|e| matches!(e, AgentEvent::PermissionRequest(_)));
    let Some(AgentEvent::PermissionRequest(req)) = evs.last() else {
        unreachable!()
    };
    h.handle
        .respond_permission(req.id, PermissionChoice::Option("no".into()))
        .unwrap();
    assert_eq!(agent_text(&h.turn()), "rechazado");
    assert!(!h.root.join("sub/nota.txt").exists());
}

#[test]
fn leer_con_linea_y_limite() {
    let h = connect(None, None);
    std::fs::write(h.root.join("datos.txt"), "a\nb\nc\nd\n").unwrap();
    h.ready();
    h.handle.prompt("leer").unwrap();
    assert_eq!(agent_text(&h.turn()), "b\nc\n");
}

#[test]
fn cancelar_descarta_permisos_pendientes() {
    let h = connect(None, None);
    h.ready();
    h.handle.prompt("esperar").unwrap();
    let evs = h.until(|e| matches!(e, AgentEvent::PermissionRequest(_)));
    let Some(AgentEvent::PermissionRequest(req)) = evs.last() else {
        unreachable!()
    };
    h.handle.cancel().unwrap();
    let evs = h.turn();
    assert!(evs.contains(&AgentEvent::PermissionCancelled(req.id)));
    assert_eq!(
        evs.last(),
        Some(&AgentEvent::TurnEnded(StopReason::Cancelled))
    );
    // Responder tarde a un permiso ya descartado no rompe nada.
    h.handle
        .respond_permission(req.id, PermissionChoice::Cancel)
        .unwrap();
    h.handle.prompt("hola").unwrap();
    assert_eq!(agent_text(&h.turn()), "Hola, mundo");
}

#[test]
fn reanudar_sesion_existente() {
    let h = connect(Some("s-vieja"), None);
    let evs = h.until(|e| matches!(e, AgentEvent::Ready(_)));
    assert!(evs.contains(&AgentEvent::UserChunk("hola de antes".into())));
    let Some(AgentEvent::Ready(r)) = evs.last() else {
        unreachable!()
    };
    assert_eq!(r.session_id, "s-vieja");
    assert!(r.resumed);
}

#[test]
fn reanudar_sesion_desconocida_abre_una_nueva() {
    let h = connect(Some("s-perdida"), None);
    let evs = h.until(|e| matches!(e, AgentEvent::Ready(_)));
    assert!(evs
        .iter()
        .any(|e| matches!(e, AgentEvent::Notice(n) if n.contains("s-perdida"))));
    let Some(AgentEvent::Ready(r)) = evs.last() else {
        unreachable!()
    };
    assert_eq!(r.session_id, "s-nueva");
    assert!(!r.resumed);
}

#[test]
fn modo_inicial_y_cambio_de_modo() {
    let h = connect(None, Some("plan"));
    h.ready();
    h.until(|e| *e == AgentEvent::ModeChanged("plan".into()));
    h.handle.set_mode("default").unwrap();
    h.until(|e| *e == AgentEvent::ModeChanged("default".into()));
}

#[test]
fn modo_inicial_inexistente_avisa() {
    let h = connect(None, Some("yolo"));
    h.ready();
    let evs = h.until(|e| matches!(e, AgentEvent::Notice(_)));
    let Some(AgentEvent::Notice(n)) = evs.last() else {
        unreachable!()
    };
    assert!(n.contains("yolo") && n.contains("default, plan"), "{n}");
}

#[test]
fn cerrar_emite_exited_sin_error() {
    let h = connect(None, None);
    h.ready();
    let Harness {
        handle, rx, _tmp, ..
    } = h;
    handle.shutdown();
    let last = rx.iter().last();
    assert_eq!(
        last,
        Some(AgentEvent::Exited {
            code: None,
            error: None
        })
    );
}

#[test]
fn programa_inexistente() {
    let tmp = tempfile::tempdir().unwrap();
    let cmd = qorvi_acp::AgentCommand {
        program: "qorvi-agente-que-no-existe".into(),
        ..Default::default()
    };
    let r = AgentHandle::spawn(
        &cmd,
        SessionSpec {
            cwd: tmp.path().to_path_buf(),
            ..Default::default()
        },
        |_| {},
    );
    assert!(matches!(r, Err(qorvi_acp::Error::Spawn { .. })));
}

#[cfg(unix)]
#[test]
fn agente_que_sale_al_arrancar() {
    let tmp = tempfile::tempdir().unwrap();
    let cmd = qorvi_acp::AgentCommand {
        program: "sh".into(),
        args: vec!["-c".into(), "echo adios >&2; exit 3".into()],
        ..Default::default()
    };
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let _h = AgentHandle::spawn(
        &cmd,
        SessionSpec {
            cwd: tmp.path().to_path_buf(),
            ..Default::default()
        },
        move |ev| {
            let _ = tx.lock().unwrap().send(ev);
        },
    )
    .unwrap();
    let mut evs = Vec::new();
    while let Ok(ev) = rx.recv_timeout(TIMEOUT) {
        let fin = matches!(ev, AgentEvent::Exited { .. });
        evs.push(ev);
        if fin {
            break;
        }
    }
    match evs.last() {
        Some(AgentEvent::Exited { code, error }) => {
            assert_eq!(*code, Some(3), "{evs:?}");
            let error = error.as_deref().unwrap();
            assert!(error.contains("código 3"), "{error}");
            assert!(error.ends_with("stderr del agente:\nadios"), "{error}");
        }
        other => panic!("esperaba Exited, llegó {other:?} ({evs:?})"),
    }
}

#[cfg(unix)]
#[test]
fn al_cerrarse_mueren_tambien_los_nietos() {
    let tmp = tempfile::tempdir().unwrap();
    let pidfile = tmp.path().join("nieto.pid");
    let cmd = qorvi_acp::AgentCommand {
        program: "sh".into(),
        // Un envoltorio que deja un nieto vivo y sale (como `npx` que muere antes que `node`).
        args: vec![
            "-c".into(),
            format!(
                "sleep 300 & echo $! > '{}'; sleep 0.2; exit 0",
                pidfile.display()
            ),
        ],
        ..Default::default()
    };
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(tx);
    let _h = AgentHandle::spawn(
        &cmd,
        SessionSpec {
            cwd: tmp.path().to_path_buf(),
            ..Default::default()
        },
        move |ev| {
            let _ = tx.lock().unwrap().send(ev);
        },
    )
    .unwrap();
    loop {
        match rx.recv_timeout(TIMEOUT) {
            Ok(AgentEvent::Exited { .. }) => break,
            Ok(_) => {}
            Err(e) => panic!("sin Exited: {e}"),
        }
    }
    let pid: i32 = std::fs::read_to_string(&pidfile)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // El nieto recibe la señal antes de `Exited`; puede tardar un instante en desaparecer.
    let vivo = || {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
            || std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .is_ok_and(|s| s.success())
    };
    let t0 = std::time::Instant::now();
    while vivo() && t0.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!vivo(), "el nieto {pid} sigue vivo");
}
