use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

/// Un proceso hijo en un PTY con su pantalla emulada por vt100.
pub struct Pane {
    pub parser: Arc<Mutex<vt100::Parser>>,
    /// Primeros 64 KiB de salida cruda, para diagnosticar consultas a la terminal.
    pub raw: Arc<Mutex<Vec<u8>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    exited: Arc<AtomicBool>,
    size: (u16, u16),
}

impl Pane {
    /// Lanza `program args` en un PTY de `rows`×`cols`. Cada vez que llega salida
    /// se envía `()` por `wake` para que la UI redibuje.
    pub fn spawn(
        program: &str,
        args: &[String],
        cwd: &Path,
        rows: u16,
        cols: u16,
        wake: Sender<()>,
    ) -> Result<Pane> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("openpty")?;

        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        cmd.cwd(cwd);
        cmd.env("TERM", "xterm-256color");
        let child = pair
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("no se pudo lanzar {program}"))?;
        // El slave ya no se necesita en este proceso; cerrarlo permite detectar EOF.
        drop(pair.slave);

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 1000)));
        let exited = Arc::new(AtomicBool::new(false));
        let raw = Arc::new(Mutex::new(Vec::new()));
        let mut reader = pair.master.try_clone_reader().context("reader")?;
        let writer = Arc::new(Mutex::new(pair.master.take_writer().context("writer")?));

        {
            let parser = Arc::clone(&parser);
            let exited = Arc::clone(&exited);
            let raw = Arc::clone(&raw);
            let writer = Arc::clone(&writer);
            thread::spawn(move || {
                let mut buf = [0u8; 8192];
                // Cola de la lectura anterior por si una consulta llega partida.
                let mut tail: Vec<u8> = Vec::new();
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if let Ok(mut r) = raw.lock() {
                                if r.len() < 65536 {
                                    r.extend_from_slice(&buf[..n]);
                                }
                            }
                            let mut cursor = (0, 0);
                            if let Ok(mut p) = parser.lock() {
                                p.process(&buf[..n]);
                                cursor = p.screen().cursor_position();
                            }
                            tail.extend_from_slice(&buf[..n]);
                            let (reply, keep) = query_replies(&tail, cursor);
                            if !reply.is_empty() {
                                if let Ok(mut w) = writer.lock() {
                                    let _ = w.write_all(&reply);
                                    let _ = w.flush();
                                }
                            }
                            tail.drain(..keep);
                            let _ = wake.send(());
                        }
                    }
                }
                exited.store(true, Ordering::SeqCst);
                let _ = wake.send(());
            });
        }

        Ok(Pane {
            parser,
            raw,
            writer,
            master: pair.master,
            child,
            exited,
            size: (rows, cols),
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let mut w = self
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("writer envenenado"))?;
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        if self.size == (rows, cols) || rows == 0 || cols == 0 {
            return Ok(());
        }
        self.size = (rows, cols);
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        if let Ok(mut p) = self.parser.lock() {
            p.screen_mut().set_size(rows, cols);
        }
        Ok(())
    }

    pub fn has_exited(&mut self) -> bool {
        self.exited.load(Ordering::SeqCst) || matches!(self.child.try_wait(), Ok(Some(_)))
    }

    pub fn contents(&self) -> String {
        self.parser
            .lock()
            .map(|p| p.screen().contents())
            .unwrap_or_default()
    }

    /// (application_cursor, bracketed_paste) del programa hijo.
    pub fn input_modes(&self) -> (bool, bool) {
        self.parser
            .lock()
            .map(|p| {
                (
                    p.screen().application_cursor(),
                    p.screen().bracketed_paste(),
                )
            })
            .unwrap_or((false, false))
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Respuestas a las consultas que los programas hacen a "su terminal" y que vt100
/// no contesta. Sin ellas, nvim y otras TUIs esperan un timeout al arrancar.
///
/// `cursor` = (fila, columna) base 0. Devuelve las respuestas y el índice de `data`
/// desde el que hay que conservar bytes para la próxima lectura (el inicio de una
/// secuencia que podría estar cortada), o `data.len()` si no hay nada pendiente.
pub fn query_replies(data: &[u8], cursor: (u16, u16)) -> (Vec<u8>, usize) {
    const ST_OSC10: &[u8] = b"\x1b]10;rgb:d0d0/d0d0/d0d0\x1b\\";
    const ST_OSC11: &[u8] = b"\x1b]11;rgb:1c1c/1c1c/1c1c\x1b\\";
    let fixed: [(&[u8], &[u8]); 8] = [
        (b"\x1b[5n", b"\x1b[0n"),
        (b"\x1b[c", b"\x1b[?62;22c"),
        (b"\x1b[0c", b"\x1b[?62;22c"),
        (b"\x1b[>c", b"\x1b[>0;10;1c"),
        (b"\x1b[>0c", b"\x1b[>0;10;1c"),
        (b"\x1b[>q", b"\x1bP>|qorvi\x1b\\"),
        (b"\x1b[>0q", b"\x1bP>|qorvi\x1b\\"),
        (b"\x1b[6n", b""), // se calcula aparte
    ];
    let mut out = Vec::new();
    let mut keep = data.len();
    let mut i = 0;
    while i < data.len() {
        if data[i] != 0x1b {
            i += 1;
            continue;
        }
        let rest = &data[i..];
        let mut matched = None;
        for (query, reply) in fixed {
            if rest.starts_with(query) {
                let reply = if query == b"\x1b[6n" {
                    format!("\x1b[{};{}R", cursor.0 + 1, cursor.1 + 1).into_bytes()
                } else {
                    reply.to_vec()
                };
                matched = Some((query.len(), reply));
                break;
            }
        }
        if matched.is_none() {
            for (code, reply) in [(b"10", ST_OSC10), (b"11", ST_OSC11)] {
                if let Some(n) = osc_query_len(rest, code) {
                    matched = Some((n, reply.to_vec()));
                    break;
                }
            }
        }
        match matched {
            Some((len, reply)) => {
                out.extend_from_slice(&reply);
                i += len;
            }
            None => {
                // Un ESC sin cerrar cerca del final puede ser una consulta cortada.
                if data.len() - i < 16 && !rest[1..].contains(&0x1b) {
                    keep = i;
                    break;
                }
                i += 1;
            }
        }
    }
    (out, keep)
}

/// Longitud de `ESC ] code ; ? (BEL | ESC \)` al principio de `s`, si está completa.
fn osc_query_len(s: &[u8], code: &[u8]) -> Option<usize> {
    let mut head = b"\x1b]".to_vec();
    head.extend_from_slice(code);
    head.extend_from_slice(b";?");
    let after = s.strip_prefix(head.as_slice())?;
    if after.starts_with(b"\x07") {
        Some(head.len() + 1)
    } else if after.starts_with(b"\x1b\\") {
        Some(head.len() + 2)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::query_replies;

    #[test]
    fn answers_common_queries() {
        assert_eq!(query_replies(b"\x1b[6n", (4, 9)).0, b"\x1b[5;10R");
        assert_eq!(query_replies(b"abc\x1b[c", (0, 0)).0, b"\x1b[?62;22c");
        assert_eq!(
            query_replies(b"\x1b]11;?\x07", (0, 0)).0,
            b"\x1b]11;rgb:1c1c/1c1c/1c1c\x1b\\"
        );
        // Kitty y DECRQM no se contestan: la app lo deduce al recibir la respuesta a DA1.
        assert_eq!(query_replies(b"\x1b[?u\x1b[c", (0, 0)).0, b"\x1b[?62;22c");
        assert!(query_replies(b"\x1b[?2026$p", (0, 0)).0.is_empty());
    }

    #[test]
    fn split_query_is_kept_and_answered_once() {
        // Primera lectura: consulta completa + inicio de otra.
        let (reply, keep) = query_replies(b"hola\x1b[c\x1b]11;", (0, 0));
        assert_eq!(reply, b"\x1b[?62;22c");
        assert_eq!(keep, 7); // se conserva solo "\x1b]11;"
                             // Segunda lectura: lo conservado + el resto. DA1 no se repite.
        let (reply, keep) = query_replies(b"\x1b]11;?\x07texto", (0, 0));
        assert_eq!(reply, b"\x1b]11;rgb:1c1c/1c1c/1c1c\x1b\\");
        assert_eq!(keep, 12); // nada pendiente
    }
}
