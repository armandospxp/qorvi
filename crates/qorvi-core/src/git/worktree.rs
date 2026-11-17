//! Operaciones de `git worktree`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use super::Git;
use crate::error::{Error, Result};

/// Un worktree tal como lo lista `git worktree list`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    /// Commit al que apunta `HEAD` (ausente en repos bare).
    pub head: Option<String>,
    /// Rama (sin el prefijo `refs/heads/`); `None` si está separado o es bare.
    pub branch: Option<String>,
    pub bare: bool,
    pub detached: bool,
    pub locked: bool,
    pub prunable: bool,
}

impl Git {
    /// Lista los worktrees; el primero es el árbol principal.
    pub fn worktree_list(&self) -> Result<Vec<WorktreeInfo>> {
        let out = self.run(["worktree", "list", "--porcelain", "-z"])?;
        parse_worktree_list(&out)
    }

    /// Crea un worktree en `path` con la rama nueva `new_branch` partiendo de `base`.
    ///
    /// Falla con [`Error::BranchExists`] si la rama ya existe y con [`Error::PathExists`]
    /// si `path` existe y no está vacío. Crea el directorio padre si falta. `base` se
    /// resuelve antes a un sha, así que nunca se interpreta como opción ni queda como
    /// upstream de la rama nueva.
    pub fn worktree_add(&self, path: &Path, new_branch: &str, base: &str) -> Result<()> {
        if self.branch_exists(new_branch)? {
            return Err(Error::BranchExists(new_branch.to_string()));
        }
        let full = self.cwd().join(path);
        if !dir_is_empty_or_missing(&full)? {
            return Err(Error::PathExists(path.to_path_buf()));
        }
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let sha = self.resolve_commit(base)?;
        self.run([
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-b"),
            OsStr::new(new_branch),
            OsStr::new("--"),
            path.as_os_str(),
            OsStr::new(&sha),
        ])
        .map(|_| ())
    }

    /// Elimina el worktree de `path` (`--force` si `force`, para cambios sin guardar).
    pub fn worktree_remove(&self, path: &Path, force: bool) -> Result<()> {
        let mut args: Vec<&OsStr> = vec![OsStr::new("worktree"), OsStr::new("remove")];
        if force {
            args.push(OsStr::new("--force"));
        }
        args.push(OsStr::new("--"));
        args.push(path.as_os_str());
        self.run(args).map(|_| ())
    }

    /// Limpia los metadatos de worktrees cuyo directorio ya no existe.
    pub fn worktree_prune(&self) -> Result<()> {
        self.run(["worktree", "prune"]).map(|_| ())
    }
}

fn dir_is_empty_or_missing(p: &Path) -> Result<bool> {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => Ok(std::fs::read_dir(p)
            .map_err(|e| Error::io(p, e))?
            .next()
            .is_none()),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(e) => Err(Error::io(p, e)),
    }
}

/// Parsea `git worktree list --porcelain -z`: campos separados por NUL y entradas
/// terminadas por un campo vacío (NUL doble).
pub(crate) fn parse_worktree_list(s: &str) -> Result<Vec<WorktreeInfo>> {
    let mut list = Vec::new();
    let mut cur: Option<WorktreeInfo> = None;
    for field in s.split('\0') {
        if field.is_empty() {
            list.extend(cur.take());
            continue;
        }
        let (key, val) = field.split_once(' ').unwrap_or((field, ""));
        if key == "worktree" {
            list.extend(cur.take());
            cur = Some(WorktreeInfo {
                path: PathBuf::from(val),
                ..Default::default()
            });
            continue;
        }
        let Some(w) = cur.as_mut() else {
            return Err(Error::GitOutput {
                args: "worktree list --porcelain -z".into(),
                detail: format!("campo `{key}` fuera de una entrada"),
            });
        };
        match key {
            "HEAD" => w.head = Some(val.to_string()),
            "branch" => w.branch = Some(val.strip_prefix("refs/heads/").unwrap_or(val).to_string()),
            "bare" => w.bare = true,
            "detached" => w.detached = true,
            "locked" => w.locked = true,
            "prunable" => w.prunable = true,
            _ => {}
        }
    }
    list.extend(cur.take());
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::super::branch::testutil::*;
    use super::*;

    #[test]
    fn parse_literal() {
        let s = "worktree /a/mi repo\0HEAD 0123\0branch refs/heads/main\0\0\
                 worktree /b/ñ\0HEAD 4567\0detached\0locked\0\0\
                 worktree /c\0HEAD 89ab\0branch refs/heads/x/y\0locked por qué\0prunable gitdir apunta a nada\0\0\
                 worktree /d.git\0bare\0\0";
        let l = parse_worktree_list(s).unwrap();
        assert_eq!(l.len(), 4);
        assert_eq!(l[0].path, PathBuf::from("/a/mi repo"));
        assert_eq!(l[0].branch.as_deref(), Some("main"));
        assert!(!l[0].locked && !l[0].detached);
        assert_eq!(l[1].path, PathBuf::from("/b/ñ"));
        assert!(l[1].detached && l[1].locked && l[1].branch.is_none());
        assert_eq!(l[2].branch.as_deref(), Some("x/y"));
        assert!(l[2].locked && l[2].prunable);
        assert!(l[3].bare && l[3].head.is_none());
        assert!(parse_worktree_list("").unwrap().is_empty());
    }

    #[test]
    fn crear_y_listar() {
        let (tmp, root) = repo();
        let g = Git::new(&root);
        let wt = tmp.path().join("nivel1").join("nivel2").join("wt");
        g.worktree_add(&wt, "tarea", "main").unwrap();
        assert!(g.branch_exists("tarea").unwrap());
        let l = g.worktree_list().unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(std::fs::canonicalize(&l[0].path).unwrap(), root);
        assert_eq!(l[0].branch.as_deref(), Some("main"));
        assert_eq!(
            std::fs::canonicalize(&l[1].path).unwrap(),
            std::fs::canonicalize(&wt).unwrap()
        );
        assert_eq!(l[1].branch.as_deref(), Some("tarea"));
        assert_eq!(
            l[1].head.as_deref(),
            Some(g.head_commit().unwrap().as_str())
        );
    }

    #[test]
    fn rama_existente() {
        let (tmp, root) = repo();
        let g = Git::new(&root);
        let err = g
            .worktree_add(&tmp.path().join("wt"), "main", "main")
            .unwrap_err();
        assert!(matches!(err, Error::BranchExists(n) if n == "main"));
    }

    #[test]
    fn ruta_no_vacia_y_vacia() {
        let (tmp, root) = repo();
        let g = Git::new(&root);
        let wt = tmp.path().join("wt");
        std::fs::create_dir(&wt).unwrap();
        std::fs::write(wt.join("x"), "x").unwrap();
        let err = g.worktree_add(&wt, "t1", "main").unwrap_err();
        assert!(matches!(err, Error::PathExists(_)));
        assert!(!g.branch_exists("t1").unwrap());
        std::fs::remove_file(wt.join("x")).unwrap();
        g.worktree_add(&wt, "t1", "main").unwrap();
    }

    #[test]
    fn base_inexistente_no_crea_nada() {
        let (tmp, root) = repo();
        let g = Git::new(&root);
        assert!(g.worktree_add(&tmp.path().join("wt"), "t", "nada").is_err());
        assert!(!g.branch_exists("t").unwrap());
    }

    #[test]
    fn quitar_y_prune() {
        let (tmp, root) = repo();
        let g = Git::new(&root);
        let wt = tmp.path().join("wt");
        g.worktree_add(&wt, "t", "main").unwrap();
        std::fs::write(wt.join("sucio.txt"), "x").unwrap();
        git(&wt, &["add", "sucio.txt"]);
        assert!(g.worktree_remove(&wt, false).is_err());
        g.worktree_remove(&wt, true).unwrap();
        assert!(!wt.exists());
        assert_eq!(g.worktree_list().unwrap().len(), 1);

        let wt2 = tmp.path().join("wt2");
        g.worktree_add(&wt2, "t2", "main").unwrap();
        std::fs::remove_dir_all(&wt2).unwrap();
        assert!(g.worktree_list().unwrap()[1].prunable);
        g.worktree_prune().unwrap();
        assert_eq!(g.worktree_list().unwrap().len(), 1);
    }
}
