//! nvim real con `--listen`: se abren archivos desde fuera con `NvimRemote`.
//! Solo se prueba si `nvim` está instalado.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use qorvi_pty::editor::editor_command;
use qorvi_pty::{Pane, PaneEvent};

const T: Duration = Duration::from_secs(15);

fn has_nvim() -> bool {
    std::process::Command::new("nvim")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn wait_for(pane: &Pane, rx: &mpsc::Receiver<PaneEvent>, what: &str, pred: impl Fn(&str) -> bool) {
    let start = Instant::now();
    while start.elapsed() < T {
        if pred(&pane.contents()) {
            return;
        }
        let _ = rx.recv_timeout(Duration::from_millis(50));
    }
    panic!("timeout esperando {what}; pantalla:\n{}", pane.contents());
}

#[test]
fn nvim_abre_archivos_desde_fuera() {
    if !has_nvim() {
        eprintln!("nvim no está instalado: se omite la prueba");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let spec = editor_command("nvim --clean", dir.path(), "test editor").unwrap();
    let remote = spec.remote.clone().unwrap();
    let (tx, rx) = mpsc::channel();
    let pane = Pane::spawn(spec.command, move |ev| {
        let _ = tx.send(ev);
    })
    .unwrap();

    let sock = PathBuf::from(remote.addr());
    let start = Instant::now();
    while !sock.exists() {
        assert!(start.elapsed() < T, "el socket {sock:?} no apareció");
        std::thread::sleep(Duration::from_millis(50));
    }

    let f1 = dir.path().join("it's a file.txt");
    std::fs::write(&f1, "contenido-unico-xyz\n").unwrap();
    remote.open_file(&f1, None).unwrap();
    wait_for(&pane, &rx, "el contenido del primer archivo", |s| {
        s.contains("contenido-unico-xyz")
    });

    let f2: &Path = &dir.path().join("largo.txt");
    let text: String = (1..=100).map(|i| format!("linea {i:03}\n")).collect();
    std::fs::write(f2, text).unwrap();
    remote.open_file(f2, Some(50)).unwrap();
    wait_for(&pane, &rx, "la línea 50", |s| s.contains("linea 050"));

    let _ = std::fs::remove_file(&sock);
}
