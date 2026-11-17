//! Descubrimiento del repositorio principal.
//!
//! qorvi siempre trabaja sobre el **árbol de trabajo principal** del repositorio, aunque se
//! lance desde un subdirectorio o desde dentro del worktree de una tarea: las tareas, la
//! config `.qorvi.toml` y los worktrees nuevos se resuelven a partir de él.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::git::Git;

/// Repositorio git descubierto a partir de un directorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// Raíz del árbol de trabajo principal (donde vive `.git`).
    pub root: PathBuf,
    /// Directorio git común a todos los worktrees (normalmente `<root>/.git`).
    pub common_dir: PathBuf,
    /// Raíz del worktree que contiene el directorio de partida. Coincide con `root`
    /// salvo que se haya partido de un worktree enlazado (p. ej. el de una tarea).
    pub current_worktree: PathBuf,
}

impl Repo {
    /// Busca el repositorio que contiene `start` y su árbol de trabajo principal.
    pub fn discover(start: impl AsRef<Path>) -> Result<Repo> {
        let start = start.as_ref();
        if !start.is_dir() {
            return Err(Error::io(
                start,
                std::io::Error::new(std::io::ErrorKind::NotFound, "el directorio no existe"),
            ));
        }
        let git = Git::new(start);

        let args = [
            "rev-parse",
            "--path-format=absolute",
            "--is-bare-repository",
            "--git-common-dir",
        ];
        let out = git.output(&args)?;
        if !out.success() {
            if out.stderr.contains("not a git repository") {
                return Err(Error::NotARepo(start.to_path_buf()));
            }
            return Err(crate::git::failed(&args, &out));
        }
        let mut lines = out.stdout.lines();
        let is_bare = lines.next() == Some("true");
        let common_dir = PathBuf::from(lines.next().ok_or_else(|| Error::GitOutput {
            args: "rev-parse --git-common-dir".into(),
            detail: "salida vacía".into(),
        })?);
        if is_bare {
            return Err(Error::BareRepo(common_dir));
        }

        let current_worktree = PathBuf::from(git.run_trim([
            "rev-parse",
            "--path-format=absolute",
            "--show-toplevel",
        ])?);

        let root = main_worktree(&git)?.ok_or_else(|| Error::BareRepo(common_dir.clone()))?;

        Ok(Repo {
            root,
            common_dir,
            current_worktree,
        })
    }

    /// `true` si se partió de un worktree enlazado y no del árbol principal.
    pub fn in_linked_worktree(&self) -> bool {
        self.current_worktree != self.root
    }

    /// Ejecutor git en la raíz del árbol principal.
    pub fn git(&self) -> Git {
        Git::new(&self.root)
    }
}

/// Ruta del árbol de trabajo principal: la primera entrada de `git worktree list`.
/// Devuelve `None` si el repositorio principal es bare.
fn main_worktree(git: &Git) -> Result<Option<PathBuf>> {
    let list = git.run(["worktree", "list", "--porcelain"])?;
    parse_main_worktree(&list).ok_or_else(|| Error::GitOutput {
        args: "worktree list --porcelain".into(),
        detail: "no aparece ningún worktree".into(),
    })
}

fn parse_main_worktree(porcelain: &str) -> Option<Option<PathBuf>> {
    let mut block = porcelain.split("\n\n").next()?.lines();
    let path = block.next()?.strip_prefix("worktree ")?;
    let bare = block.any(|l| l == "bare");
    Some((!bare).then(|| PathBuf::from(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        Git::new(dir).run(args).unwrap();
    }

    /// Repo con un commit inicial (los worktrees necesitan HEAD).
    fn repo_con_commit() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        std::fs::write(root.join("a.txt"), "a").unwrap();
        git(&root, &["add", "."]);
        git(
            &root,
            &[
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "init",
            ],
        );
        // Rutas reales (en macOS /var → /private/var) para comparar con lo que da git.
        let root = std::fs::canonicalize(&root).unwrap();
        (tmp, root)
    }

    fn canon(p: &Path) -> PathBuf {
        std::fs::canonicalize(p).unwrap()
    }

    #[test]
    fn desde_la_raiz() {
        let (_tmp, root) = repo_con_commit();
        let repo = Repo::discover(&root).unwrap();
        assert_eq!(canon(&repo.root), root);
        assert_eq!(canon(&repo.common_dir), root.join(".git"));
        assert!(!repo.in_linked_worktree());
    }

    #[test]
    fn desde_un_subdirectorio() {
        let (_tmp, root) = repo_con_commit();
        let sub = root.join("x").join("y");
        std::fs::create_dir_all(&sub).unwrap();
        let repo = Repo::discover(&sub).unwrap();
        assert_eq!(canon(&repo.root), root);
        assert_eq!(canon(&repo.current_worktree), root);
    }

    #[test]
    fn desde_un_worktree_enlazado() {
        let (tmp, root) = repo_con_commit();
        let wt = tmp.path().join("wt");
        git(
            &root,
            &["worktree", "add", "-q", "-b", "tarea", wt.to_str().unwrap()],
        );
        let repo = Repo::discover(&wt).unwrap();
        assert_eq!(canon(&repo.root), root);
        assert_eq!(canon(&repo.common_dir), root.join(".git"));
        assert_eq!(canon(&repo.current_worktree), canon(&wt));
        assert!(repo.in_linked_worktree());
    }

    #[test]
    fn fuera_de_un_repo() {
        let tmp = tempfile::tempdir().unwrap();
        // Evita que un repo en un directorio padre del tmp (raro, pero posible) interfiera.
        let out = Git::new(tmp.path())
            .output(&["rev-parse", "--git-dir"])
            .unwrap();
        if out.success() {
            return;
        }
        match Repo::discover(tmp.path()) {
            Err(Error::NotARepo(p)) => assert_eq!(p, tmp.path()),
            other => panic!("resultado inesperado: {other:?}"),
        }
    }

    #[test]
    fn repo_bare() {
        let tmp = tempfile::tempdir().unwrap();
        git(tmp.path(), &["init", "-q", "--bare"]);
        assert!(matches!(
            Repo::discover(tmp.path()),
            Err(Error::BareRepo(_))
        ));
    }

    #[test]
    fn directorio_inexistente() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(matches!(
            Repo::discover(tmp.path().join("no")),
            Err(Error::Io { .. })
        ));
    }

    #[test]
    fn parse_porcelain() {
        let s = "worktree /a/b\nHEAD 0123\nbranch refs/heads/main\n\nworktree /c\nHEAD 4567\ndetached\n\n";
        assert_eq!(parse_main_worktree(s), Some(Some(PathBuf::from("/a/b"))));
        assert_eq!(parse_main_worktree("worktree /a.git\nbare\n\n"), Some(None));
        assert_eq!(parse_main_worktree(""), None);
    }
}
