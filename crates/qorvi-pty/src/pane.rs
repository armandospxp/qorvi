//! Panel de terminal: un proceso hijo en un PTY con su pantalla emulada por `vt100`.
//!
//! Cada panel usa dos hilos:
//! - **lector**: lee la salida del PTY, la pasa al parser, escribe las respuestas a las
//!   consultas de terminal ([`Responder`]) y avisa con [`PaneEvent`];
//! - **espera**: bloquea en `wait()` del hijo y avisa cuando termina. Va aparte porque en
//!   Windows (ConPTY) la salida no llega a EOF cuando el hijo termina.
//!
//! La UI no lee del PTY: dibuja con [`Pane::with_screen`] cuando recibe un evento.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use crossterm::event::KeyEvent;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};

use crate::keys::{encode_key, encode_paste};
use crate::responder::Responder;

/// Errores de un panel.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No se pudo abrir el PTY.
    #[error("no se pudo abrir una pseudo-terminal: {0}")]
    OpenPty(String),
    /// No se pudo lanzar el programa.
    #[error("no se pudo lanzar `{program}`: {message}")]
    Spawn { program: String, message: String },
    /// Fallo de E/S con el PTY.
    #[error("error de E/S con la terminal del panel: {0}")]
    Io(#[from] std::io::Error),
    /// El programa del panel ya terminó.
    #[error("el programa del panel ya terminó")]
    Exited,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Tamaño de un panel en celdas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSize {
    pub rows: u16,
    pub cols: u16,
}

impl PaneSize {
    pub fn new(rows: u16, cols: u16) -> PaneSize {
        PaneSize {
            rows: rows.max(1),
            cols: cols.max(1),
        }
    }

    fn pty(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

/// Qué lanzar en un panel.
#[derive(Debug, Clone)]
pub struct PaneCommand {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    /// Variables extra (p. ej. `QORVI_TASK`). Tienen prioridad sobre las heredadas.
    pub env: Vec<(OsString, OsString)>,
    pub size: PaneSize,
    /// Líneas de historial que se conservan al salir de la pantalla.
    pub scrollback: usize,
}

impl PaneCommand {
    /// Comando sin argumentos, 24×80 y 10 000 líneas de historial.
    pub fn new(program: impl Into<OsString>, cwd: impl Into<PathBuf>) -> PaneCommand {
        PaneCommand {
            program: program.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            env: Vec::new(),
            size: PaneSize::new(24, 80),
            scrollback: 10_000,
        }
    }

    pub fn args<I, S>(mut self, args: I) -> PaneCommand
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> PaneCommand {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn size(mut self, size: PaneSize) -> PaneCommand {
        self.size = size;
        self
    }

    pub fn scrollback(mut self, lines: usize) -> PaneCommand {
        self.scrollback = lines;
        self
    }
}

/// Variables de la terminal anfitriona que no deben llegar al programa del panel: le
/// harían creer que corre directamente en esa terminal (gráficos de kitty, integración de
/// iTerm2…) o en un nvim (un nvim anidado intentaría conectarse al de fuera).
const HOST_ENV: &[&str] = &[
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "TERM_SESSION_ID",
    "KITTY_WINDOW_ID",
    "KITTY_PID",
    "KITTY_PUBLIC_KEY",
    "KITTY_INSTALLATION_DIR",
    "ITERM_SESSION_ID",
    "ITERM_PROFILE",
    "LC_TERMINAL",
    "LC_TERMINAL_VERSION",
    "GHOSTTY_RESOURCES_DIR",
    "GHOSTTY_BIN_DIR",
    "WEZTERM_PANE",
    "WEZTERM_EXECUTABLE",
    "WEZTERM_UNIX_SOCKET",
    "ALACRITTY_WINDOW_ID",
    "ALACRITTY_SOCKET",
    "VTE_VERSION",
    "WT_SESSION",
    "WT_PROFILE_ID",
    "TERMINAL_EMULATOR",
    "VSCODE_INJECTION",
    "NVIM",
    "NVIM_LISTEN_ADDRESS",
    "COLUMNS",
    "LINES",
];

/// Evento de un panel. Se entrega desde los hilos del panel: el callback debe ser rápido
/// (p. ej. enviar a un canal) y no bloquear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneEvent {
    /// Hay salida nueva: la pantalla cambió.
    Output,
    /// El programa cambió el título (OSC 0/2).
    Title(String),
    /// Sonó la campana (útil para avisar de tareas en segundo plano).
    Bell,
    /// El programa terminó.
    Exited(ExitStatus),
}

/// Cómo terminó el programa del panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitStatus {
    pub code: u32,
    /// Señal que lo terminó (solo Unix), p. ej. `"Killed"`.
    pub signal: Option<String>,
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.code == 0 && self.signal.is_none()
    }
}

type Notify = Arc<dyn Fn(PaneEvent) + Send + Sync>;

/// Un programa en una pseudo-terminal. Al soltarlo se mata el programa.
pub struct Pane {
    parser: Arc<Mutex<vt100::Parser<Responder>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    exit: Arc<Mutex<Option<ExitStatus>>>,
    pid: Option<u32>,
    size: PaneSize,
}

impl std::fmt::Debug for Pane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pane")
            .field("pid", &self.pid)
            .field("size", &self.size)
            .field("exit", &self.exit_status())
            .finish_non_exhaustive()
    }
}

impl Pane {
    /// Lanza `cmd` en un PTY nuevo. `notify` recibe los [`PaneEvent`] del panel.
    pub fn spawn(
        cmd: PaneCommand,
        notify: impl Fn(PaneEvent) + Send + Sync + 'static,
    ) -> Result<Pane> {
        let notify: Notify = Arc::new(notify);
        let size = PaneSize::new(cmd.size.rows, cmd.size.cols);
        let pair = native_pty_system()
            .openpty(size.pty())
            .map_err(|e| Error::OpenPty(e.to_string()))?;

        let program = cmd.program.to_string_lossy().into_owned();
        let mut builder = CommandBuilder::new(&cmd.program);
        builder.args(&cmd.args);
        builder.cwd(&cmd.cwd);
        for var in HOST_ENV {
            builder.env_remove(var);
        }
        builder.env("TERM", "xterm-256color");
        builder.env("COLORTERM", "truecolor");
        builder.env("TERM_PROGRAM", "qorvi");
        builder.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        for (k, v) in &cmd.env {
            builder.env(k, v);
        }

        let mut child = pair
            .slave
            .spawn_command(builder)
            .map_err(|e| Error::Spawn {
                program: program.clone(),
                message: e.to_string(),
            })?;
        // El slave ya no hace falta aquí; cerrarlo permite detectar EOF en Unix.
        drop(pair.slave);

        let pid = child.process_id();
        let killer = child.clone_killer();
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::OpenPty(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::OpenPty(e.to_string()))?;
        let writer = Arc::new(Mutex::new(writer));
        let parser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(
            size.rows,
            size.cols,
            cmd.scrollback,
            Responder::default(),
        )));
        let exit = Arc::new(Mutex::new(None));

        spawn_reader(
            reader,
            Arc::clone(&parser),
            Arc::clone(&writer),
            Arc::clone(&notify),
        );
        {
            let exit = Arc::clone(&exit);
            let notify = Arc::clone(&notify);
            thread::Builder::new()
                .name(format!("qorvi-pty-wait-{program}"))
                .spawn(move || {
                    let status = match child.wait() {
                        Ok(s) => ExitStatus {
                            code: s.exit_code(),
                            signal: s.signal().map(str::to_string),
                        },
                        Err(_) => ExitStatus {
                            code: 1,
                            signal: None,
                        },
                    };
                    *lock(&exit) = Some(status.clone());
                    notify(PaneEvent::Exited(status));
                })?;
        }

        Ok(Pane {
            parser,
            writer,
            master: pair.master,
            killer,
            exit,
            pid,
            size,
        })
    }

    /// Escribe bytes crudos al programa.
    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        if self.exit_status().is_some() {
            return Err(Error::Exited);
        }
        let mut w = lock(&self.writer);
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    }

    /// Envía una tecla, codificada según los modos que pidió el programa (p. ej. cursor
    /// de aplicación en nvim). Devuelve `false` si la tecla no tiene codificación.
    pub fn send_key(&self, key: KeyEvent) -> Result<bool> {
        let app_cursor = self.with_screen(|s| s.application_cursor());
        match encode_key(key, app_cursor) {
            Some(bytes) => self.write(&bytes).map(|_| true),
            None => Ok(false),
        }
    }

    /// Pega texto (con bracketed paste si el programa lo activó).
    pub fn paste(&self, text: &str) -> Result<()> {
        let bracketed = self.with_screen(|s| s.bracketed_paste());
        self.write(&encode_paste(text, bracketed))
    }

    /// Cambia el tamaño del PTY y de la pantalla emulada. El programa recibe `SIGWINCH`.
    pub fn resize(&mut self, size: PaneSize) -> Result<()> {
        let size = PaneSize::new(size.rows, size.cols);
        if size == self.size {
            return Ok(());
        }
        // Primero la pantalla: así la salida que el programa genere tras SIGWINCH ya se
        // interpreta con el tamaño nuevo.
        lock(&self.parser)
            .screen_mut()
            .set_size(size.rows, size.cols);
        self.master
            .resize(size.pty())
            .map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;
        self.size = size;
        Ok(())
    }

    /// Tamaño actual.
    pub fn size(&self) -> PaneSize {
        self.size
    }

    /// Acceso a la pantalla emulada (para dibujarla). No guardes el lock: el lector del
    /// panel espera mientras dura `f`.
    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        f(lock(&self.parser).screen())
    }

    /// Acceso mutable a la pantalla (p. ej. para desplazar el historial).
    pub fn with_screen_mut<R>(&self, f: impl FnOnce(&mut vt100::Screen) -> R) -> R {
        f(lock(&self.parser).screen_mut())
    }

    /// Texto visible de la pantalla.
    pub fn contents(&self) -> String {
        self.with_screen(|s| s.contents())
    }

    /// Título que puso el programa, si alguno.
    pub fn title(&self) -> Option<String> {
        lock(&self.parser).callbacks().title().map(str::to_string)
    }

    /// Estado de salida si el programa ya terminó.
    pub fn exit_status(&self) -> Option<ExitStatus> {
        lock(&self.exit).clone()
    }

    /// PID del programa, si la plataforma lo da.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Mata el programa. No falla si ya había terminado.
    pub fn kill(&mut self) -> Result<()> {
        if self.exit_status().is_some() {
            return Ok(());
        }
        match self.killer.kill() {
            Ok(()) => Ok(()),
            Err(_) if self.exit_status().is_some() => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}

fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    parser: Arc<Mutex<vt100::Parser<Responder>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    notify: Notify,
) {
    let _ = thread::Builder::new()
        .name("qorvi-pty-read".into())
        .spawn(move || {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let (replies, title, bell) = {
                    let mut p = lock(&parser);
                    p.process(&buf[..n]);
                    let cb = p.callbacks_mut();
                    (cb.take_replies(), cb.take_title_change(), cb.take_bell())
                };
                if !replies.is_empty() {
                    let mut w = lock(&writer);
                    let _ = w.write_all(&replies);
                    let _ = w.flush();
                }
                if let Some(t) = title {
                    notify(PaneEvent::Title(t));
                }
                if bell {
                    notify(PaneEvent::Bell);
                }
                notify(PaneEvent::Output);
            }
        });
}

/// Lock que sobrevive a un panic en otro hilo: la pantalla sigue siendo válida.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
