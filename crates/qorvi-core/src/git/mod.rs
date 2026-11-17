//! Ejecutor de comandos `git`.
//!
//! Todas las operaciones git de qorvi pasan por [`Git`]: lanza el binario `git` como lista
//! de argumentos (nunca a través de una shell), sin prompts interactivos y con una salida
//! estable para poder analizarla.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::error::{Error, Result};

mod branch;
mod commit;
mod diff;
mod merge;
mod remote;
mod status;
mod worktree;

#[cfg(test)]
pub(crate) use branch::testutil;
pub use diff::{ChangeKind, FileChange};
pub use merge::MergeStyle;
pub use status::{FileState, FileStatus};
pub use worktree::WorktreeInfo;

/// Ejecuta `git` en un directorio fijo.
#[derive(Debug, Clone)]
pub struct Git {
    cwd: PathBuf,
}

/// Salida de un comando git que terminó (con éxito o no).
#[derive(Debug, Clone)]
pub struct GitOutput {
    /// Código de salida, o `None` si el proceso terminó por una señal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl GitOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

impl Git {
    /// Crea un ejecutor que lanza `git` con `cwd` como directorio de trabajo.
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Git { cwd: cwd.into() }
    }

    /// Directorio en el que se ejecuta `git`.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Ejecuta `git <args>` y devuelve su stdout. Falla si el código de salida no es 0.
    pub fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<_> = args.into_iter().collect();
        let out = self.output(&args)?;
        if out.success() {
            Ok(out.stdout)
        } else {
            Err(failed(&args, &out))
        }
    }

    /// Como [`Git::run`], pero quita el salto de línea final de stdout.
    pub fn run_trim<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut s = self.run(args)?;
        let len = s.trim_end_matches(['\n', '\r']).len();
        s.truncate(len);
        Ok(s)
    }

    /// Ejecuta `git <args>` y devuelve la salida sin interpretar el código de salida.
    ///
    /// Útil para comandos donde un código distinto de 0 es una respuesta válida
    /// (p. ej. `git diff --quiet` o `git show-ref --verify`).
    pub fn output<S: AsRef<OsStr>>(&self, args: &[S]) -> Result<GitOutput> {
        let out = self
            .command(args)
            .output()
            .map_err(|e| spawn_error(args, e))?;
        to_git_output(args, out)
    }

    /// Como [`Git::run`], pero pasa `input` por stdin (útil para mensajes con `-F -`).
    pub fn run_with_stdin<I, S>(&self, args: I, input: &str) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<_> = args.into_iter().collect();
        let out = self.output_with_stdin(&args, input)?;
        if out.success() {
            Ok(out.stdout)
        } else {
            Err(failed(&args, &out))
        }
    }

    /// Como [`Git::output`], pero pasa `input` por stdin.
    pub fn output_with_stdin<S: AsRef<OsStr>>(&self, args: &[S], input: &str) -> Result<GitOutput> {
        let mut cmd = self.command(args);
        let out = output_with_stdin(&mut cmd, input).map_err(|e| spawn_error(args, e))?;
        to_git_output(args, out)
    }

    /// Construye el `Command` con el entorno estándar de qorvi.
    fn command<S: AsRef<OsStr>>(&self, args: &[S]) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.cwd)
            // Rutas sin escapar (UTF-8 tal cual) en la salida de status/diff/ls-files.
            .args(["-c", "core.quotepath=false"])
            .args(args)
            .stdin(Stdio::null())
            // Nunca pedir credenciales ni abrir un editor: qorvi no tiene dónde mostrarlos.
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", "true")
            .env("GIT_PAGER", "cat")
            // Mensajes en inglés y estables para poder analizarlos.
            .env("LC_ALL", "C")
            .env("LANGUAGE", "C");
        // Evitar que un GIT_DIR/GIT_WORK_TREE heredado apunte a otro repositorio.
        for var in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
        ] {
            cmd.env_remove(var);
        }
        cmd
    }
}

/// Lanza `cmd` escribiendo `input` por stdin (desde un hilo, para no bloquearse con salidas
/// grandes) y espera a que termine.
pub(crate) fn output_with_stdin(cmd: &mut Command, input: &str) -> std::io::Result<Output> {
    use std::io::Write;
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin entubado");
    let data = input.as_bytes().to_vec();
    let writer = std::thread::spawn(move || {
        // Si el proceso termina sin leer todo, el error de tubería se ignora a propósito.
        let _ = stdin.write_all(&data);
    });
    let out = child.wait_with_output();
    let _ = writer.join();
    out
}

fn display_args<S: AsRef<OsStr>>(args: &[S]) -> String {
    args.iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn spawn_error<S: AsRef<OsStr>>(args: &[S], e: std::io::Error) -> Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        Error::GitNotFound
    } else {
        Error::GitSpawn {
            args: display_args(args),
            source: e,
        }
    }
}

fn to_git_output<S: AsRef<OsStr>>(args: &[S], out: Output) -> Result<GitOutput> {
    let utf8 = |bytes: Vec<u8>, stream: &str| {
        String::from_utf8(bytes).map_err(|_| Error::GitOutput {
            args: display_args(args),
            detail: format!("{stream} no es UTF-8 válido"),
        })
    };
    Ok(GitOutput {
        code: out.status.code(),
        stdout: utf8(out.stdout, "stdout")?,
        // stderr solo se usa en mensajes: una conversión con pérdida es suficiente.
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

pub(crate) fn failed<S: AsRef<OsStr>>(args: &[S], out: &GitOutput) -> Error {
    Error::GitFailed {
        args: display_args(args),
        code: out.code,
        stderr: out.stderr.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        Git::new(dir.path()).run(["init", "-q"]).unwrap();
        dir
    }

    #[test]
    fn run_devuelve_stdout() {
        let dir = init_repo();
        let out = Git::new(dir.path())
            .run_trim(["rev-parse", "--is-inside-work-tree"])
            .unwrap();
        assert_eq!(out, "true");
    }

    #[test]
    fn run_falla_con_codigo_y_stderr() {
        let dir = init_repo();
        let err = Git::new(dir.path())
            .run(["rev-parse", "--verify", "no-existe"])
            .unwrap_err();
        match err {
            Error::GitFailed {
                args, code, stderr, ..
            } => {
                assert_eq!(args, "rev-parse --verify no-existe");
                assert_eq!(code, Some(128));
                assert!(!stderr.is_empty());
            }
            other => panic!("error inesperado: {other:?}"),
        }
    }

    #[test]
    fn output_no_falla_con_codigo_distinto_de_cero() {
        let dir = init_repo();
        let out = Git::new(dir.path())
            .output(&["rev-parse", "--verify", "--quiet", "HEAD"])
            .unwrap();
        assert!(!out.success());
    }

    #[test]
    fn run_with_stdin_pasa_la_entrada() {
        let dir = init_repo();
        let out = Git::new(dir.path())
            .run_with_stdin(["hash-object", "--stdin"], "hola\n")
            .unwrap();
        assert_eq!(out.trim().len(), 40);
    }

    #[test]
    fn rutas_no_ascii_sin_escapar() {
        let dir = init_repo();
        std::fs::write(dir.path().join("canción.txt"), "la").unwrap();
        let out = Git::new(dir.path()).run(["ls-files", "--others"]).unwrap();
        assert_eq!(out.trim(), "canción.txt");
    }
}
