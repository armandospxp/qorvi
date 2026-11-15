//! Comprobaciones automáticas sin terminal real: lanza programas en un PTY, les
//! envía teclas y verifica la pantalla emulada por vt100.

use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::pane::Pane;

fn wait_for(
    pane: &mut Pane,
    what: &str,
    timeout: Duration,
    pred: impl Fn(&str) -> bool,
) -> Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if pred(&pane.contents()) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    bail!("timeout esperando {what}; pantalla:\n{}", pane.contents())
}

fn spawn(program: &str, args: &[&str], cwd: &Path, rows: u16, cols: u16) -> Result<Pane> {
    let (tx, _rx) = mpsc::channel();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    Pane::spawn(program, &args, cwd, rows, cols, tx)
}

type Check = fn(&Path) -> Result<()>;

pub fn run_all() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("pty-spike-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let checks: &[(&str, Check)] = &[
        ("shell: eco y aritmética", check_shell),
        ("shell: Ctrl+C interrumpe sleep", check_ctrl_c),
        ("shell: resize llega al hijo (stty size)", check_resize),
        ("nvim: insertar, guardar y salir", check_nvim_edit),
        ("nvim: modo application cursor y flechas", check_nvim_arrows),
        ("less: paginar y salir con q", check_less),
        ("colores ANSI 256 y truecolor", check_colors),
    ];
    let mut failed = 0;
    for (name, f) in checks {
        let start = Instant::now();
        match f(&dir) {
            Ok(()) => println!("OK    {name} ({} ms)", start.elapsed().as_millis()),
            Err(e) => {
                failed += 1;
                println!("FALLO {name}: {e:#}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    if failed > 0 {
        bail!("{failed} comprobaciones fallaron");
    }
    Ok(())
}

const T: Duration = Duration::from_secs(5);

fn check_shell(dir: &Path) -> Result<()> {
    let mut p = spawn("/bin/sh", &[], dir, 24, 80)?;
    p.write(b"echo resultado=$((40+2))\r")?;
    wait_for(&mut p, "resultado=42", T, |s| s.contains("resultado=42"))
}

fn check_ctrl_c(dir: &Path) -> Result<()> {
    let mut p = spawn("/bin/sh", &[], dir, 24, 80)?;
    p.write(b"sleep 100\r")?;
    thread::sleep(Duration::from_millis(300));
    p.write(&[0x03])?;
    p.write(b"echo sigo-vivo\r")?;
    wait_for(&mut p, "sigo-vivo", T, |s| {
        s.lines().any(|l| l.trim() == "sigo-vivo")
    })
}

fn check_resize(dir: &Path) -> Result<()> {
    let mut p = spawn("/bin/sh", &[], dir, 24, 80)?;
    p.resize(30, 100)?;
    p.write(b"stty size\r")?;
    wait_for(&mut p, "30 100", T, |s| s.contains("30 100"))
}

fn check_nvim_edit(dir: &Path) -> Result<()> {
    let file = dir.join("nota.txt");
    let mut p = spawn(
        "nvim",
        &["--clean", "-n", file.to_str().unwrap_or("nota.txt")],
        dir,
        24,
        80,
    )?;
    wait_for(&mut p, "nvim arrancado", T, |s| s.contains("nota.txt"))?;
    p.write(b"ihola qorvi")?;
    wait_for(&mut p, "texto insertado", T, |s| s.contains("hola qorvi"))?;
    p.write(b"\x1b:wq\r")?;
    let start = Instant::now();
    while !p.has_exited() {
        if start.elapsed() > T {
            bail!("nvim no salió; pantalla:\n{}", p.contents());
        }
        thread::sleep(Duration::from_millis(20));
    }
    let saved = std::fs::read_to_string(&file)?;
    if saved != "hola qorvi\n" {
        bail!("contenido guardado inesperado: {saved:?}");
    }
    Ok(())
}

fn check_nvim_arrows(dir: &Path) -> Result<()> {
    let file = dir.join("lineas.txt");
    std::fs::write(&file, "uno\ndos\ntres\n")?;
    let mut p = spawn(
        "nvim",
        &["--clean", "-n", file.to_str().unwrap_or("lineas.txt")],
        dir,
        24,
        80,
    )?;
    wait_for(&mut p, "nvim arrancado", T, |s| s.contains("tres"))?;
    let (app_cursor, _) = p.input_modes();
    let down = crate::keys::encode_key(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        ),
        app_cursor,
    )
    .unwrap_or_default();
    // Bajar dos líneas con la flecha y borrar la línea: debe desaparecer "tres".
    p.write(&down)?;
    p.write(&down)?;
    p.write(b"dd")?;
    wait_for(&mut p, "línea 'tres' borrada", T, |s| {
        !s.contains("tres") && s.contains("dos")
    })?;
    p.write(b":q!\r")?;
    Ok(())
}

fn check_less(dir: &Path) -> Result<()> {
    let file = dir.join("largo.txt");
    let body: String = (1..=200).map(|i| format!("linea-{i}\n")).collect();
    std::fs::write(&file, body)?;
    let mut p = spawn("less", &[file.to_str().unwrap_or("largo.txt")], dir, 24, 80)?;
    wait_for(&mut p, "primera página", T, |s| {
        s.contains("linea-1\n") || s.starts_with("linea-1")
    })?;
    p.write(b" ")?;
    wait_for(&mut p, "segunda página", T, |s| {
        s.contains("linea-30") && !s.contains("linea-1\n")
    })?;
    p.write(b"q")?;
    let start = Instant::now();
    while !p.has_exited() {
        if start.elapsed() > T {
            bail!("less no salió");
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

fn check_colors(dir: &Path) -> Result<()> {
    let mut p = spawn("/bin/sh", &[], dir, 24, 80)?;
    p.write(b"printf '\\033[38;5;208mNARANJA\\033[0m \\033[38;2;10;20;30mTRUE\\033[0m\\n'\r")?;
    wait_for(&mut p, "texto coloreado", T, |s| s.contains("NARANJA TRUE"))?;
    let parser = p
        .parser
        .lock()
        .map_err(|_| anyhow::anyhow!("mutex envenenado"))?;
    let screen = parser.screen();
    // Buscar la celda de 'N' de la salida (no la del comando tecleado).
    let (rows, cols) = screen.size();
    let mut found = (None, None);
    for r in 0..rows {
        let row: String = (0..cols)
            .filter_map(|c| screen.cell(r, c).map(|x| x.contents().to_string()))
            .collect();
        if let Some(col) = row.find("NARANJA TRUE") {
            if !row.contains("printf") {
                found.0 = screen.cell(r, col as u16).map(|c| c.fgcolor());
                found.1 = screen.cell(r, col as u16 + 8).map(|c| c.fgcolor());
            }
        }
    }
    match found {
        (Some(vt100::Color::Idx(208)), Some(vt100::Color::Rgb(10, 20, 30))) => Ok(()),
        other => bail!("colores inesperados: {other:?}"),
    }
}

/// Lanza `program` y lista las consultas a la terminal que emite en sus primeros
/// `secs` segundos (vt100 no las responde).
pub fn queries(program: &str, args: &[String], secs: u64) -> Result<()> {
    let (tx, _rx) = mpsc::channel();
    let cwd = std::env::current_dir()?;
    let p = Pane::spawn(program, args, &cwd, 30, 100, tx)?;
    thread::sleep(Duration::from_secs(secs));
    let raw = p.raw.lock().map_err(|_| anyhow::anyhow!("mutex"))?.clone();
    let text = String::from_utf8_lossy(&raw);
    let known = [
        ("\x1b[6n", "DSR posición del cursor"),
        ("\x1b[c", "DA1 atributos primarios"),
        ("\x1b[0c", "DA1 atributos primarios"),
        ("\x1b[>c", "DA2 atributos secundarios"),
        ("\x1b[>0q", "XTVERSION"),
        ("\x1b]10;?", "OSC 10 color de texto"),
        ("\x1b]11;?", "OSC 11 color de fondo"),
        ("\x1b[?u", "protocolo de teclado kitty"),
        ("\x1b[?2026$p", "DECRQM synchronized output"),
        ("\x1bP+q", "XTGETTCAP"),
    ];
    println!("{program}: {} bytes de salida", raw.len());
    for (seq, name) in known {
        let n = text.matches(seq).count();
        if n > 0 {
            println!("  {n}× {name} ({:?})", seq);
        }
    }
    println!("pantalla:\n{}", p.contents());
    Ok(())
}
