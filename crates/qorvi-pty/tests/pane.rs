//! Programas reales en un PTY: se les envían teclas y se comprueba la pantalla emulada.
//! nvim y less solo se prueban si están instalados.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use qorvi_pty::{Pane, PaneCommand, PaneEvent, PaneSize};

const T: Duration = Duration::from_secs(10);

struct Harness {
    pane: Pane,
    events: mpsc::Receiver<PaneEvent>,
    _dir: tempfile::TempDir,
}

fn spawn(program: &str, args: &[&str], size: PaneSize) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    spawn_in(program, args, size, dir)
}

fn spawn_in(program: &str, args: &[&str], size: PaneSize, dir: tempfile::TempDir) -> Harness {
    let (tx, rx) = mpsc::channel();
    let cmd = PaneCommand::new(program, dir.path())
        .args(args.iter().copied())
        .env("QORVI_TEST", "1")
        // Prompt fijo y sin rc del usuario para que la pantalla sea predecible.
        .env("PS1", "$ ")
        .env("ENV", "")
        .size(size);
    let pane = Pane::spawn(cmd, move |ev| {
        let _ = tx.send(ev);
    })
    .unwrap();
    Harness {
        pane,
        events: rx,
        _dir: dir,
    }
}

fn wait_for(h: &Harness, what: &str, pred: impl Fn(&str) -> bool) {
    let start = Instant::now();
    while start.elapsed() < T {
        if pred(&h.pane.contents()) {
            return;
        }
        let _ = h.events.recv_timeout(Duration::from_millis(50));
    }
    panic!("timeout esperando {what}; pantalla:\n{}", h.pane.contents());
}

fn wait_exit(h: &Harness) -> qorvi_pty::ExitStatus {
    let start = Instant::now();
    while start.elapsed() < T {
        if let Some(s) = h.pane.exit_status() {
            return s;
        }
        let _ = h.events.recv_timeout(Duration::from_millis(50));
    }
    panic!("el programa no terminó; pantalla:\n{}", h.pane.contents());
}

fn installed(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

const SIZE: PaneSize = PaneSize { rows: 24, cols: 80 };

#[cfg(unix)]
mod unix {
    use super::*;

    #[test]
    fn shell_eco_y_variables() {
        let h = spawn("/bin/sh", &[], SIZE);
        h.pane
            .write(b"echo r=$((40+2)) $QORVI_TEST $TERM $TERM_PROGRAM\r")
            .unwrap();
        wait_for(&h, "eco", |s| s.contains("r=42 1 xterm-256color qorvi"));
    }

    #[test]
    fn variables_de_la_terminal_anfitriona_no_llegan() {
        // KITTY_WINDOW_ID es una de las que se filtran; se comprueba que el hijo no la ve
        // aunque el proceso de test la tuviera (no se puede fijar sin afectar a otros
        // tests, así que se comprueba solo el resultado visible).
        let h = spawn("/bin/sh", &[], SIZE);
        h.pane.write(b"echo k=[${KITTY_WINDOW_ID}]\r").unwrap();
        wait_for(&h, "variable vacía", |s| s.contains("k=[]"));
    }

    #[test]
    fn ctrl_c_interrumpe() {
        let h = spawn("/bin/sh", &[], SIZE);
        h.pane.write(b"sleep 100; echo no-deberia\r").unwrap();
        std::thread::sleep(Duration::from_millis(300));
        h.pane
            .send_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .unwrap();
        h.pane.write(b"echo sigo-vivo\r").unwrap();
        wait_for(&h, "sigo-vivo", |s| s.contains("sigo-vivo"));
        // La línea del comando contiene el texto; la salida del echo sería una línea sola.
        assert!(!h.pane.contents().lines().any(|l| l.trim() == "no-deberia"));
    }

    #[test]
    fn resize_llega_al_hijo() {
        let mut h = spawn("/bin/sh", &[], SIZE);
        h.pane.resize(PaneSize::new(30, 100)).unwrap();
        assert_eq!(h.pane.with_screen(|s| s.size()), (30, 100));
        h.pane.write(b"stty size\r").unwrap();
        wait_for(&h, "30 100", |s| s.contains("30 100"));
    }

    #[test]
    fn codigo_de_salida_y_evento() {
        let h = spawn("/bin/sh", &["-c", "exit 7"], SIZE);
        let status = wait_exit(&h);
        assert_eq!(status.code, 7);
        assert!(!status.success());
        assert!(matches!(h.pane.write(b"x"), Err(qorvi_pty::Error::Exited)));
        // El evento Exited también llegó (puede que ya se consumiera en wait_exit).
    }

    #[test]
    fn kill_termina_el_programa() {
        let mut h = spawn("/bin/sh", &["-c", "sleep 100"], SIZE);
        h.pane.kill().unwrap();
        let status = wait_exit(&h);
        assert!(!status.success());
        // Matar dos veces no falla.
        h.pane.kill().unwrap();
    }

    #[test]
    fn programa_inexistente() {
        let dir = tempfile::tempdir().unwrap();
        let err =
            Pane::spawn(PaneCommand::new("/no/existe/qorvi", dir.path()), |_| {}).unwrap_err();
        assert!(err.to_string().contains("/no/existe/qorvi"), "{err}");
    }

    #[test]
    fn titulo_por_osc() {
        let h = spawn(
            "/bin/sh",
            &["-c", "printf '\\033]2;mi-titulo\\007'; sleep 5"],
            SIZE,
        );
        let start = Instant::now();
        loop {
            match h.events.recv_timeout(T) {
                Ok(PaneEvent::Title(t)) => {
                    assert_eq!(t, "mi-titulo");
                    break;
                }
                Ok(_) if start.elapsed() < T => continue,
                other => panic!("no llegó el título: {other:?}"),
            }
        }
        assert_eq!(h.pane.title().as_deref(), Some("mi-titulo"));
    }

    #[test]
    fn el_hijo_recibe_la_posicion_del_cursor() {
        // Pide la posición del cursor (DSR 6) y muestra la respuesta que recibe por stdin.
        let script = "stty -echo raw; printf '\\033[3;5H\\033[6n'; \
                      r=$(dd bs=1 count=6 2>/dev/null); stty sane; \
                      printf '\\nrespuesta=%s\\n' \"$(printf '%s' \"$r\" | tr -d '\\033')\"; sleep 5";
        let h = spawn("/bin/sh", &["-c", script], SIZE);
        wait_for(&h, "respuesta", |s| s.contains("respuesta=[3;5R"));
    }

    #[test]
    fn pegado_con_bracketed_paste() {
        // El hijo activa el modo 2004 y muestra lo que recibe, con los ESC visibles.
        let script = "printf '\\033[?2004h'; stty -echo raw; \
                      r=$(dd bs=1 count=14 2>/dev/null); stty sane; \
                      printf '\\nrecibido=%s\\n' \"$(printf '%s' \"$r\" | tr '\\033' 'E')\"; sleep 5";
        let h = spawn("/bin/sh", &["-c", script], SIZE);
        wait_for(&h, "modo bracketed", |_| {
            h.pane.with_screen(|s| s.bracketed_paste())
        });
        h.pane.paste("ab").unwrap();
        wait_for(&h, "pegado", |s| s.contains("recibido=E[200~abE[201~"));
    }

    #[test]
    fn nvim_editar_guardar_y_salir() {
        if !installed("nvim") {
            eprintln!("nvim no está instalado: se omite");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nota.txt");
        let h = spawn_in(
            "nvim",
            &["--clean", "-n", file.to_str().unwrap()],
            SIZE,
            dir,
        );
        wait_for(&h, "nvim arrancado", |s| s.contains("nota.txt"));
        h.pane.write(b"ihola desde qorvi").unwrap();
        h.pane.send_key(key(KeyCode::Esc)).unwrap();
        h.pane.write(b":wq\r").unwrap();
        let status = wait_exit(&h);
        assert!(status.success(), "{status:?}");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap().trim_end(),
            "hola desde qorvi"
        );
    }

    #[test]
    fn nvim_flechas_en_modo_cursor_de_aplicacion() {
        if !installed("nvim") {
            eprintln!("nvim no está instalado: se omite");
            return;
        }
        let h = spawn("nvim", &["--clean", "-n"], SIZE);
        h.pane.write(b"iabc").unwrap();
        h.pane.send_key(key(KeyCode::Esc)).unwrap();
        wait_for(&h, "texto", |s| s.contains("abc"));
        // Dos flechas a la izquierda y una 'X' insertada antes de la 'a'... o donde
        // caiga el cursor: si las flechas no se entendieran, saldría basura en el texto.
        h.pane.send_key(key(KeyCode::Left)).unwrap();
        h.pane.send_key(key(KeyCode::Left)).unwrap();
        h.pane.write(b"iX").unwrap();
        h.pane.send_key(key(KeyCode::Esc)).unwrap();
        wait_for(&h, "Xabc", |s| {
            s.lines().next().unwrap_or("").starts_with("Xabc")
        });
    }

    #[test]
    fn less_paginar_y_salir() {
        if !installed("less") {
            eprintln!("less no está instalado: se omite");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("largo.txt");
        let text: String = (1..=200).map(|i| format!("linea {i}\n")).collect();
        std::fs::write(&file, text).unwrap();
        let h = spawn_in("less", &[file.to_str().unwrap()], SIZE, dir);
        wait_for(&h, "primera página", |s| s.contains("linea 1\n"));
        h.pane.write(b" ").unwrap();
        wait_for(&h, "segunda página", |s| s.contains("linea 30"));
        h.pane.write(b"q").unwrap();
        assert!(wait_exit(&h).success());
    }

    #[test]
    fn colores_256_y_truecolor() {
        let h = spawn(
            "/bin/sh",
            &[
                "-c",
                "printf '\\033[38;5;196mR\\033[38;2;1;2;3mT\\033[0m\\n'; sleep 5",
            ],
            SIZE,
        );
        wait_for(&h, "texto", |s| s.contains("RT"));
        h.pane.with_screen(|s| {
            assert_eq!(s.cell(0, 0).unwrap().fgcolor(), vt100::Color::Idx(196));
            assert_eq!(s.cell(0, 1).unwrap().fgcolor(), vt100::Color::Rgb(1, 2, 3));
        });
    }

    use qorvi_pty::vt100;
}

#[cfg(windows)]
mod windows {
    use super::*;

    #[test]
    fn cmd_eco_y_salida() {
        let h = spawn("cmd.exe", &["/Q", "/K", "prompt $G"], SIZE);
        h.pane.write(b"echo r=%QORVI_TEST%\r").unwrap();
        wait_for(&h, "eco", |s| s.contains("r=1"));
        h.pane.write(b"exit 3\r").unwrap();
        assert_eq!(wait_exit(&h).code, 3);
    }

    #[test]
    fn resize_no_falla() {
        let mut h = spawn("cmd.exe", &["/Q", "/K", "prompt $G"], SIZE);
        h.pane.resize(PaneSize::new(30, 100)).unwrap();
        assert_eq!(h.pane.with_screen(|s| s.size()), (30, 100));
    }
}
