//! Tests de integración: ejecutan el binario contra un repositorio temporal.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use tempfile::TempDir;

struct Env {
    _tmp: TempDir,
    repo: PathBuf,
    config: PathBuf,
    data: PathBuf,
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

fn env() -> Env {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    let config = tmp.path().join("config");
    let data = tmp.path().join("data");
    for d in [&repo, &config, &data] {
        std::fs::create_dir_all(d).unwrap();
    }
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "inicial"]);
    std::fs::write(
        config.join("config.toml"),
        "default_agent = \"fake\"\n[agents.fake]\ncommand = \"fake\"\n",
    )
    .unwrap();
    Env {
        _tmp: tmp,
        repo,
        config,
        data,
    }
}

impl Env {
    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_qorvi"));
        c.args(args)
            .current_dir(&self.repo)
            .env("QORVI_CONFIG_DIR", &self.config)
            .env("QORVI_DATA_DIR", &self.data)
            .stdin(Stdio::null());
        c
    }
    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().unwrap()
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn flujo_new_ls_rm() {
    let e = env();
    let o = e.run(&["new", "uno"]);
    assert!(o.status.success(), "{}", err(&o));
    let s = out(&o);
    assert!(s.contains("Tarea uno creada"), "{s}");
    assert!(s.contains("rama uno") && s.contains("base main"), "{s}");
    assert!(s.contains("cd "), "{s}");

    let o = e.run(&["ls"]);
    assert!(o.status.success());
    let s = out(&o);
    assert!(s.starts_with("NOMBRE"), "{s}");
    assert!(s.contains("uno") && s.contains("fake") && s.contains("limpio"));

    let o = e.run(&["ls", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v[0]["name"], "uno");
    assert_eq!(v[0]["agent"], "fake");
    assert_eq!(v[0]["changes"], 0);

    let o = e.run(&["rm", "uno", "--yes"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("borrada"));
    let o = e.run(&["ls"]);
    assert!(out(&o).contains("No hay tareas. Crea una con: qorvi new <nombre>"));
    let o = e.run(&["ls", "--json"]);
    assert_eq!(out(&o).trim().replace(char::is_whitespace, ""), "[]");
}

#[test]
fn print_path_imprime_solo_la_ruta() {
    let e = env();
    let o = e.run(&["new", "uno", "--print-path"]);
    assert!(o.status.success(), "{}", err(&o));
    let path = PathBuf::from(out(&o).trim());
    assert!(path.is_dir());
    assert!(path.ends_with("uno"));
    assert_eq!(out(&o).lines().count(), 1);
}

#[test]
fn rm_con_cambios_falla() {
    let e = env();
    let o = e.run(&["new", "uno", "--print-path"]);
    let path = PathBuf::from(out(&o).trim());
    std::fs::write(path.join("nuevo.txt"), "x").unwrap();

    let o = e.run(&["ls"]);
    assert!(out(&o).contains("1 cambio"), "{}", out(&o));
    let o = e.run(&["ls", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(v[0]["changes"], 1);

    let o = e.run(&["rm", "uno", "--yes"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).starts_with("error: "), "{}", err(&o));
    assert!(path.is_dir());

    let o = e.run(&["rm", "uno", "--yes", "--force"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(!path.exists());
}

#[test]
fn rm_sin_yes_y_sin_tty_falla() {
    let e = env();
    e.run(&["new", "uno"]);
    let o = e.run(&["rm", "uno"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("--yes"), "{}", err(&o));
    assert!(out(&e.run(&["ls"])).contains("uno"));
}

#[test]
fn new_con_nombre_invalido() {
    let e = env();
    let o = e.run(&["new", "mal nombre"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).starts_with("error: "), "{}", err(&o));
}

#[test]
fn opcion_c_usa_otro_directorio() {
    let e = env();
    let repo = e.repo.to_str().unwrap().to_string();
    let mut c = e.cmd(&["-C", &repo, "new", "uno"]);
    c.current_dir(&e.config);
    let o = c.output().unwrap();
    assert!(o.status.success(), "{}", err(&o));
    let mut c = e.cmd(&["ls", "-C", &repo]);
    c.current_dir(&e.config);
    assert!(out(&c.output().unwrap()).contains("uno"));
}

#[test]
fn huerfanos() {
    let e = env();
    let o = e.run(&["new", "uno", "--print-path"]);
    let dir = PathBuf::from(out(&o).trim())
        .parent()
        .unwrap()
        .to_path_buf();
    let orphan = dir.join("huerfano");
    git(
        &e.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "huerfano",
            orphan.to_str().unwrap(),
        ],
    );

    let o = e.run(&["ls"]);
    assert!(err(&o).contains("huerfano"), "{}", err(&o));
    assert!(err(&o).contains("qorvi rm --orphans"));
    let o = e.run(&["ls", "--json"]);
    serde_json::from_str::<serde_json::Value>(&out(&o)).unwrap();

    let o = e.run(&["rm", "--orphans"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(orphan.is_dir());

    let o = e.run(&["rm", "--orphans", "--yes"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(!orphan.exists());
    assert!(e.run(&["ls"]).stderr.is_empty());
}

#[test]
fn sin_subcomando_imprime_version() {
    let e = env();
    let o = e.run(&[]);
    assert!(o.status.success());
    assert!(out(&o).starts_with("qorvi "));
}
