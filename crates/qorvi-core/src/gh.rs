//! Ejecutor de GitHub CLI (`gh`).
//!
//! Análogo a [`crate::Git`]: lanza `gh` como lista de argumentos (sin shell), sin stdin
//! interactivo ni prompts, y con salida sin colores.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::git::output_with_stdin;

/// Datos de un pull request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrInfo {
    pub number: u64,
    pub url: String,
    /// `OPEN`, `CLOSED` o `MERGED`.
    pub state: String,
    pub is_draft: bool,
}

/// Ejecuta `gh` en un directorio fijo.
#[derive(Debug, Clone)]
pub struct Gh {
    cwd: PathBuf,
    program: String,
}

struct GhOut {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Gh {
    /// Crea un ejecutor que lanza `gh` con `cwd` como directorio de trabajo.
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self::with_program(cwd, "gh")
    }

    /// Como [`Gh::new`] pero con otro nombre de binario (para tests).
    fn with_program(cwd: impl Into<PathBuf>, program: &str) -> Self {
        Gh {
            cwd: cwd.into(),
            program: program.to_string(),
        }
    }

    /// Directorio en el que se ejecuta `gh`.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// `true` si hay una sesión de `gh` iniciada (`gh auth status` con código 0).
    pub fn is_authenticated(&self) -> Result<bool> {
        Ok(self.output(&["auth", "status"], None)?.code == Some(0))
    }

    /// Crea un PR de `head` hacia `base` y devuelve su URL. El cuerpo va por stdin.
    pub fn create_pr(
        &self,
        base: &str,
        head: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<String> {
        for r in [base, head] {
            if r.is_empty() || r.starts_with('-') {
                return Err(Error::InvalidRef(r.to_string()));
            }
        }
        let mut args = vec![
            "pr",
            "create",
            "--base",
            base,
            "--head",
            head,
            "--title",
            title,
            "--body-file",
            "-",
        ];
        if draft {
            args.push("--draft");
        }
        let out = self.output(&args, Some(body))?;
        if out.code != Some(0) {
            return Err(failed(&args, &out));
        }
        parse_pr_url(&out.stdout).ok_or_else(|| Error::GhOutput {
            args: display(&args),
            detail: format!("no se encontró la URL del PR en {:?}", out.stdout),
        })
    }

    /// PR asociado a `branch`, o `None` si no tiene.
    pub fn pr_for_branch(&self, branch: &str) -> Result<Option<PrInfo>> {
        if branch.is_empty() || branch.starts_with('-') {
            return Err(Error::InvalidRef(branch.to_string()));
        }
        let args = ["pr", "view", branch, "--json", "number,url,state,isDraft"];
        let out = self.output(&args, None)?;
        if out.code == Some(0) {
            return parse_pr_info(&out.stdout)
                .map(Some)
                .map_err(|detail| Error::GhOutput {
                    args: display(&args),
                    detail,
                });
        }
        if out.stderr.contains("no pull requests found") {
            return Ok(None);
        }
        Err(failed(&args, &out))
    }

    fn output<S: AsRef<OsStr>>(&self, args: &[S], input: Option<&str>) -> Result<GhOut> {
        let mut cmd = Command::new(&self.program);
        cmd.current_dir(&self.cwd)
            .args(args)
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_COLOR", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1");
        let res = match input {
            Some(input) => output_with_stdin(&mut cmd, input),
            None => cmd.stdin(Stdio::null()).output(),
        };
        let out: Output = res.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::GhNotFound
            } else {
                Error::GhSpawn {
                    args: display(args),
                    source: e,
                }
            }
        })?;
        Ok(GhOut {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

fn failed<S: AsRef<OsStr>>(args: &[S], out: &GhOut) -> Error {
    Error::GhFailed {
        args: display(args),
        code: out.code,
        stderr: out.stderr.trim().to_string(),
    }
}

fn display<S: AsRef<OsStr>>(args: &[S]) -> String {
    args.iter()
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Última línea de `stdout` que empieza por `https://`.
fn parse_pr_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("https://"))
        .map(str::to_string)
}

fn parse_pr_info(json: &str) -> std::result::Result<PrInfo, String> {
    serde_json::from_str(json).map_err(|e| format!("JSON inválido: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_es_la_ultima_linea_https() {
        let out =
            "\nCreating pull request for x into main in o/r\n\nhttps://github.com/o/r/pull/7\n";
        assert_eq!(
            parse_pr_url(out).as_deref(),
            Some("https://github.com/o/r/pull/7")
        );
        assert_eq!(parse_pr_url("sin url\nhttp://no\n"), None);
        assert_eq!(
            parse_pr_url("https://a/1\nhttps://a/2\nfin").as_deref(),
            Some("https://a/2")
        );
    }

    #[test]
    fn parsea_pr_view() {
        let j =
            r#"{"isDraft":true,"number":42,"state":"OPEN","url":"https://github.com/o/r/pull/42"}"#;
        assert_eq!(
            parse_pr_info(j).unwrap(),
            PrInfo {
                number: 42,
                url: "https://github.com/o/r/pull/42".into(),
                state: "OPEN".into(),
                is_draft: true,
            }
        );
        assert!(parse_pr_info("{}").is_err());
        assert!(parse_pr_info("no json").is_err());
    }

    #[test]
    fn gh_no_encontrado() {
        let dir = tempfile::tempdir().unwrap();
        let gh = Gh::with_program(dir.path(), "qorvi-gh-que-no-existe");
        assert!(matches!(gh.is_authenticated(), Err(Error::GhNotFound)));
        assert!(matches!(
            gh.create_pr("main", "x", "t", "b", false),
            Err(Error::GhNotFound)
        ));
        assert!(matches!(gh.pr_for_branch("x"), Err(Error::GhNotFound)));
    }

    #[test]
    fn refs_invalidas() {
        let dir = tempfile::tempdir().unwrap();
        let gh = Gh::with_program(dir.path(), "qorvi-gh-que-no-existe");
        assert!(matches!(gh.pr_for_branch("--x"), Err(Error::InvalidRef(_))));
        assert!(matches!(
            gh.create_pr("-b", "x", "t", "b", false),
            Err(Error::InvalidRef(_))
        ));
    }
}
