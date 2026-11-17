//! Remotos: listado, push y borrado de ramas remotas.

use super::{failed, Git, GitOutput};
use crate::error::{Error, Result};

impl Git {
    /// Nombres de los remotos configurados.
    pub fn remotes(&self) -> Result<Vec<String>> {
        let out = self.run(["remote"])?;
        Ok(out.lines().map(str::to_string).collect())
    }

    /// URL configurada del remoto `name`, o `None` si no existe.
    pub fn remote_url(&self, name: &str) -> Result<Option<String>> {
        let args = ["config", "--get", &format!("remote.{name}.url")];
        let out = self.output(&args)?;
        match out.code {
            Some(0) => Ok(Some(out.stdout.trim_end_matches(['\n', '\r']).to_string())),
            Some(1) => Ok(None),
            _ => Err(failed(&args, &out)),
        }
    }

    /// Publica la rama local `branch` en `remote` (`push [-u] -- remote refs/heads/b:refs/heads/b`).
    ///
    /// Si falla por autenticación devuelve [`Error::PushAuth`]. Nunca pide credenciales por
    /// terminal (`GIT_TERMINAL_PROMPT=0`).
    pub fn push(&self, remote: &str, branch: &str, set_upstream: bool) -> Result<()> {
        let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
        let mut args = vec!["push"];
        if set_upstream {
            args.push("-u");
        }
        args.extend(["--", remote, &refspec]);
        self.run_push(&args, remote)
    }

    /// Borra la rama `branch` del remoto `remote`.
    pub fn delete_remote_branch(&self, remote: &str, branch: &str) -> Result<()> {
        let refspec = format!(":refs/heads/{branch}");
        self.run_push(&["push", "--", remote, &refspec], remote)
    }

    /// Rama upstream de `branch` (p. ej. `origin/main`), o `None` si no tiene.
    pub fn upstream_of(&self, branch: &str) -> Result<Option<String>> {
        // `@{upstream}` exige el nombre corto; se rechaza lo que parezca una opción.
        if branch.starts_with('-') {
            return Err(Error::InvalidRef(branch.to_string()));
        }
        let args = [
            "rev-parse",
            "--abbrev-ref",
            &format!("{branch}@{{upstream}}"),
        ];
        let out = self.output(&args)?;
        match out.code {
            Some(0) => Ok(Some(out.stdout.trim_end_matches(['\n', '\r']).to_string())),
            Some(128) if out.stderr.contains("no upstream") => Ok(None),
            _ => Err(failed(&args, &out)),
        }
    }

    fn run_push(&self, args: &[&str], remote: &str) -> Result<()> {
        let out = self.output(args)?;
        if out.success() {
            return Ok(());
        }
        if is_auth_failure(&out) {
            return Err(Error::PushAuth {
                remote: remote.to_string(),
                stderr: out.stderr.trim().to_string(),
            });
        }
        Err(failed(args, &out))
    }
}

fn is_auth_failure(out: &GitOutput) -> bool {
    [
        "Authentication failed",
        "could not read Username",
        "could not read Password",
        "Permission denied (publickey)",
    ]
    .iter()
    .any(|m| out.stderr.contains(m))
}

#[cfg(test)]
mod tests {
    use super::super::branch::testutil::*;
    use super::*;

    /// Repo con un remoto bare local llamado `origin`.
    fn con_remoto() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let (t, root) = repo();
        let bare = t.path().join("remoto.git");
        std::fs::create_dir(&bare).unwrap();
        git(&bare, &["init", "-q", "--bare", "-b", "main"]);
        git(&root, &["remote", "add", "origin", bare.to_str().unwrap()]);
        (t, root, bare)
    }

    #[test]
    fn remotos_y_url() {
        let (_t, root, bare) = con_remoto();
        let g = Git::new(&root);
        assert_eq!(g.remotes().unwrap(), vec!["origin"]);
        assert_eq!(g.remote_url("origin").unwrap().as_deref(), bare.to_str());
        assert_eq!(g.remote_url("otro").unwrap(), None);
    }

    #[test]
    fn sin_remotos() {
        let (_t, root) = repo();
        assert!(Git::new(&root).remotes().unwrap().is_empty());
    }

    #[test]
    fn push_sin_upstream_y_con_upstream() {
        let (_t, root, bare) = con_remoto();
        let g = Git::new(&root);
        git(&root, &["branch", "a"]);
        git(&root, &["branch", "b"]);
        g.push("origin", "a", false).unwrap();
        assert_eq!(g.upstream_of("a").unwrap(), None);
        g.push("origin", "b", true).unwrap();
        assert_eq!(g.upstream_of("b").unwrap().as_deref(), Some("origin/b"));
        let bare_git = Git::new(&bare);
        assert!(bare_git.branch_exists("a").unwrap());
        assert!(bare_git.branch_exists("b").unwrap());
    }

    #[test]
    fn push_de_rama_inexistente_falla() {
        let (_t, root, _bare) = con_remoto();
        let err = Git::new(&root).push("origin", "nada", false).unwrap_err();
        assert!(matches!(err, Error::GitFailed { .. }));
    }

    #[test]
    fn borrar_rama_remota() {
        let (_t, root, bare) = con_remoto();
        let g = Git::new(&root);
        git(&root, &["branch", "x"]);
        g.push("origin", "x", false).unwrap();
        assert!(Git::new(&bare).branch_exists("x").unwrap());
        g.delete_remote_branch("origin", "x").unwrap();
        assert!(!Git::new(&bare).branch_exists("x").unwrap());
    }

    #[test]
    fn deteccion_de_autenticacion() {
        let mk = |s: &str| GitOutput {
            code: Some(128),
            stdout: String::new(),
            stderr: s.into(),
        };
        assert!(is_auth_failure(&mk("fatal: Authentication failed for 'x'")));
        assert!(is_auth_failure(&mk(
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled"
        )));
        assert!(is_auth_failure(&mk(
            "git@github.com: Permission denied (publickey)."
        )));
        assert!(!is_auth_failure(&mk("rejected: non-fast-forward")));
    }
}
