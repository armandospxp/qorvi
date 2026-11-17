//! Operaciones de ramas y commits.

use super::{failed, Git};
use crate::error::Result;

impl Git {
    /// Rama actual, o `None` si `HEAD` está separado.
    pub fn current_branch(&self) -> Result<Option<String>> {
        let args = ["symbolic-ref", "--quiet", "--short", "HEAD"];
        let out = self.output(&args)?;
        match out.code {
            Some(0) => Ok(Some(out.stdout.trim_end_matches(['\n', '\r']).to_string())),
            Some(1) => Ok(None),
            _ => Err(failed(&args, &out)),
        }
    }

    /// Sha completo de `HEAD`.
    pub fn head_commit(&self) -> Result<String> {
        self.resolve_commit("HEAD")
    }

    /// Resuelve `rev` al sha completo del commit al que apunta. Falla si no existe.
    pub fn resolve_commit(&self, rev: &str) -> Result<String> {
        self.run_trim([
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{rev}^{{commit}}"),
        ])
    }

    /// `true` si existe la rama local `name`.
    pub fn branch_exists(&self, name: &str) -> Result<bool> {
        let args = [
            "show-ref".to_string(),
            "--verify".into(),
            "--quiet".into(),
            format!("refs/heads/{name}"),
        ];
        let out = self.output(&args)?;
        match out.code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(failed(&args, &out)),
        }
    }

    /// Rama por defecto: la de `refs/remotes/origin/HEAD` (sin `origin/`) si existe;
    /// si no, la rama actual del árbol (o `None` con `HEAD` separado).
    pub fn default_branch(&self) -> Result<Option<String>> {
        let args = [
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ];
        let out = self.output(&args)?;
        match out.code {
            Some(0) => {
                let s = out.stdout.trim_end_matches(['\n', '\r']);
                Ok(Some(s.strip_prefix("origin/").unwrap_or(s).to_string()))
            }
            Some(1) => self.current_branch(),
            _ => Err(failed(&args, &out)),
        }
    }

    /// `true` si `name` es un nombre de rama válido.
    pub fn is_valid_branch_name(&self, name: &str) -> Result<bool> {
        let args = ["check-ref-format", "--branch", name];
        let out = self.output(&args)?;
        Ok(out.success())
    }

    /// Borra la rama local `name` (`-D` si `force`, `-d` si no).
    pub fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        let flag = if force { "-D" } else { "-d" };
        self.run(["branch", flag, "--", name]).map(|_| ())
    }

    /// `true` si `branch` ya está integrada en `target` (es su ancestro).
    pub fn is_merged_into(&self, branch: &str, target: &str) -> Result<bool> {
        let a = self.resolve_commit(branch)?;
        let b = self.resolve_commit(target)?;
        let args = ["merge-base", "--is-ancestor", &a, &b];
        let out = self.output(&args)?;
        match out.code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(failed(&args, &out)),
        }
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::{Path, PathBuf};

    use crate::git::Git;

    pub fn git(dir: &Path, args: &[&str]) {
        let mut a = vec!["-c", "user.name=t", "-c", "user.email=t@t"];
        a.extend_from_slice(args);
        Git::new(dir).run(a).unwrap();
    }

    pub fn commit(dir: &Path, msg: &str) {
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-qm", msg]);
    }

    /// Repo temporal con un commit inicial en `main`. Ruta canonicalizada.
    pub fn repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        commit(&root, "init");
        let root = std::fs::canonicalize(&root).unwrap();
        (tmp, root)
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use crate::error::Error;

    #[test]
    fn rama_actual_y_head() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        assert_eq!(g.current_branch().unwrap().as_deref(), Some("main"));
        let sha = g.head_commit().unwrap();
        assert_eq!(sha.len(), 40);
        git(&root, &["checkout", "-q", "--detach"]);
        assert_eq!(g.current_branch().unwrap(), None);
        assert_eq!(g.resolve_commit("main").unwrap(), sha);
    }

    #[test]
    fn resolve_commit_falla_si_no_existe() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        assert!(matches!(
            g.resolve_commit("no-existe"),
            Err(Error::GitFailed { .. })
        ));
        assert!(g.resolve_commit("--version").is_err());
    }

    #[test]
    fn existencia_validez_y_borrado() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        assert!(g.branch_exists("main").unwrap());
        assert!(!g.branch_exists("otra").unwrap());
        assert!(g.is_valid_branch_name("qorvi/tarea-1").unwrap());
        assert!(!g.is_valid_branch_name("a b").unwrap());
        assert!(!g.is_valid_branch_name("a..b").unwrap());
        git(&root, &["branch", "otra"]);
        assert!(g.branch_exists("otra").unwrap());
        g.delete_branch("otra", false).unwrap();
        assert!(!g.branch_exists("otra").unwrap());
    }

    #[test]
    fn borrado_no_forzado_exige_fusion() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        git(&root, &["checkout", "-q", "-b", "x"]);
        std::fs::write(root.join("b.txt"), "b").unwrap();
        commit(&root, "b");
        git(&root, &["checkout", "-q", "main"]);
        assert!(g.delete_branch("x", false).is_err());
        g.delete_branch("x", true).unwrap();
    }

    #[test]
    fn merged_into() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        git(&root, &["checkout", "-q", "-b", "x"]);
        std::fs::write(root.join("b.txt"), "b").unwrap();
        commit(&root, "b");
        assert!(g.is_merged_into("main", "x").unwrap());
        assert!(!g.is_merged_into("x", "main").unwrap());
        git(&root, &["checkout", "-q", "main"]);
        git(&root, &["merge", "-q", "--ff-only", "x"]);
        assert!(g.is_merged_into("x", "main").unwrap());
    }

    #[test]
    fn default_branch_sin_remoto() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        assert_eq!(g.default_branch().unwrap().as_deref(), Some("main"));
    }

    #[test]
    fn default_branch_con_origin_head() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        git(&root, &["update-ref", "refs/remotes/origin/trunk", "HEAD"]);
        git(
            &root,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/trunk",
            ],
        );
        assert_eq!(g.default_branch().unwrap().as_deref(), Some("trunk"));
    }
}
