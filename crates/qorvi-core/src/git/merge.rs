//! Integración de ramas: merge, squash y fast-forward.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::commit::{check_message, identity_error};
use super::{failed, Git, GitOutput};
use crate::error::{Error, Result};
use crate::git::FileState;

/// Estilo de integración de una rama en la de destino.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MergeStyle {
    /// Commit de merge siempre (`--no-ff`).
    Merge,
    /// Un único commit con todos los cambios (`--squash` + commit).
    Squash,
    /// Solo avance rápido (`--ff-only`).
    FastForward,
}

impl Git {
    /// Integra `branch` en la rama actual del árbol donde se ejecuta; devuelve el sha resultante.
    ///
    /// Precondiciones (si fallan no se toca nada): `HEAD` no separado
    /// ([`Error::DetachedHead`]), árbol limpio salvo archivos sin seguimiento
    /// ([`Error::DirtyWorktree`]) y que la rama exista ([`Error::BranchNotFound`]).
    /// `message` se usa en `Merge` y `Squash` (no puede estar vacío); `FastForward` lo ignora.
    ///
    /// Si hay conflictos se aborta la operación, el árbol queda como estaba y se devuelve
    /// [`Error::MergeConflict`]. Si `FastForward` no es posible, [`Error::NotFastForward`].
    pub fn merge_branch(&self, branch: &str, style: MergeStyle, message: &str) -> Result<String> {
        if self.current_branch()?.is_none() {
            return Err(Error::DetachedHead);
        }
        if self
            .status()?
            .iter()
            .any(|f| f.unstaged != Some(FileState::Untracked))
        {
            return Err(Error::DirtyWorktree(self.cwd().to_path_buf()));
        }
        if !self.branch_exists(branch)? {
            return Err(Error::BranchNotFound(branch.to_string()));
        }
        if style != MergeStyle::FastForward {
            check_message(message)?;
        }
        // Ref completa: no puede confundirse con una opción.
        let full = format!("refs/heads/{branch}");

        match style {
            MergeStyle::Squash => {
                let args = ["merge", "--squash", full.as_str()];
                let out = self.output(&args)?;
                if !out.success() {
                    return Err(self.merge_failure(&args, &out, style)?);
                }
                if self.output(&["diff", "--cached", "--quiet"])?.success() {
                    return Err(Error::NothingToCommit);
                }
                self.commit_staged(message)
            }
            MergeStyle::Merge => {
                let args = [
                    "merge",
                    "--no-ff",
                    "--no-edit",
                    "-m",
                    message,
                    full.as_str(),
                ];
                let out = self.output(&args)?;
                if !out.success() {
                    return Err(self.merge_failure(&args, &out, style)?);
                }
                self.head_commit()
            }
            MergeStyle::FastForward => {
                let args = ["merge", "--ff-only", full.as_str()];
                let out = self.output(&args)?;
                if !out.success() {
                    return Err(self.merge_failure(&args, &out, style)?);
                }
                self.head_commit()
            }
        }
    }

    /// Analiza un merge fallido: aborta si hay conflictos y devuelve el error adecuado.
    fn merge_failure(&self, args: &[&str], out: &GitOutput, style: MergeStyle) -> Result<Error> {
        let files = self.conflicted_files()?;
        if !files.is_empty() {
            if style == MergeStyle::Squash {
                self.run(["reset", "--merge"])?;
            } else {
                self.run(["merge", "--abort"])?;
            }
            return Ok(Error::MergeConflict { files });
        }
        if style == MergeStyle::FastForward && out.stderr.contains("fast-forward") {
            return Ok(Error::NotFastForward);
        }
        Ok(identity_error(out).unwrap_or_else(|| failed(args, out)))
    }

    fn conflicted_files(&self) -> Result<Vec<PathBuf>> {
        let out = self.run(["diff", "--name-only", "--diff-filter=U", "-z"])?;
        Ok(out
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect())
    }

    /// `true` si integrar `branch` en `target` produciría conflictos. No toca el árbol ni el
    /// índice: usa `git merge-tree --write-tree`, que requiere git >= 2.38.
    pub fn has_conflicts_with(&self, branch: &str, target: &str) -> Result<bool> {
        let t = self.resolve_commit(target)?;
        let b = self.resolve_commit(branch)?;
        let args = [
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            &t,
            &b,
        ];
        let out = self.output(&args)?;
        match out.code {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(failed(&args, &out)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::branch::testutil::*;
    use super::*;
    use std::path::Path;

    fn escribir(root: &Path, f: &str, c: &str) {
        std::fs::write(root.join(f), c).unwrap();
    }

    /// `main` con `a.txt`; rama `x` con dos commits que añaden `b.txt` y `c.txt`.
    fn con_rama() -> (tempfile::TempDir, PathBuf) {
        let (t, root) = repo();
        git(&root, &["checkout", "-q", "-b", "x"]);
        escribir(&root, "b.txt", "b\n");
        commit(&root, "b");
        escribir(&root, "c.txt", "c\n");
        commit(&root, "c");
        git(&root, &["checkout", "-q", "main"]);
        (t, root)
    }

    /// `x` y `main` divergentes con conflicto en `a.txt`.
    fn con_conflicto() -> (tempfile::TempDir, PathBuf) {
        let (t, root) = repo();
        git(&root, &["checkout", "-q", "-b", "x"]);
        escribir(&root, "a.txt", "x\n");
        escribir(&root, "solo-x.txt", "x\n");
        commit(&root, "x");
        git(&root, &["checkout", "-q", "main"]);
        escribir(&root, "a.txt", "main\n");
        commit(&root, "main");
        (t, root)
    }

    fn identidad(g: &Git) {
        g.run(["config", "user.name", "t"]).unwrap();
        g.run(["config", "user.email", "t@t"]).unwrap();
    }

    fn padres(g: &Git) -> usize {
        g.run_trim(["rev-list", "--parents", "-n1", "HEAD"])
            .unwrap()
            .split(' ')
            .count()
            - 1
    }

    #[test]
    fn merge_no_ff_crea_commit_de_merge() {
        let (_t, root) = con_rama();
        let g = Git::new(&root);
        identidad(&g);
        let sha = g.merge_branch("x", MergeStyle::Merge, "integra x").unwrap();
        assert_eq!(sha, g.head_commit().unwrap());
        assert_eq!(padres(&g), 2);
        assert_eq!(
            g.run_trim(["log", "-1", "--format=%s"]).unwrap(),
            "integra x"
        );
        assert!(root.join("c.txt").exists());
    }

    #[test]
    fn merge_squash_un_solo_commit() {
        let (_t, root) = con_rama();
        let g = Git::new(&root);
        identidad(&g);
        let antes = g.head_commit().unwrap();
        let sha = g.merge_branch("x", MergeStyle::Squash, "todo x").unwrap();
        assert_ne!(sha, antes);
        assert_eq!(padres(&g), 1);
        assert_eq!(g.run_trim(["rev-parse", "HEAD~1"]).unwrap(), antes);
        assert_eq!(g.run_trim(["log", "-1", "--format=%s"]).unwrap(), "todo x");
        assert!(root.join("b.txt").exists() && root.join("c.txt").exists());
        assert!(g.is_clean().unwrap());
    }

    #[test]
    fn merge_fast_forward() {
        let (_t, root) = con_rama();
        let g = Git::new(&root);
        let x = g.resolve_commit("x").unwrap();
        let sha = g.merge_branch("x", MergeStyle::FastForward, "").unwrap();
        assert_eq!(sha, x);
        assert_eq!(g.head_commit().unwrap(), x);
    }

    #[test]
    fn fast_forward_imposible() {
        let (_t, root) = con_conflicto();
        let g = Git::new(&root);
        let antes = g.head_commit().unwrap();
        let err = g
            .merge_branch("x", MergeStyle::FastForward, "")
            .unwrap_err();
        assert!(matches!(err, Error::NotFastForward), "{err:?}");
        assert_eq!(g.head_commit().unwrap(), antes);
        assert!(g.is_clean().unwrap());
    }

    #[test]
    fn conflicto_aborta_y_deja_el_arbol_limpio() {
        for style in [MergeStyle::Merge, MergeStyle::Squash] {
            let (_t, root) = con_conflicto();
            let g = Git::new(&root);
            identidad(&g);
            let antes = g.head_commit().unwrap();
            let err = g.merge_branch("x", style, "msg").unwrap_err();
            match err {
                Error::MergeConflict { files } => {
                    assert_eq!(files, vec![PathBuf::from("a.txt")], "{style:?}")
                }
                other => panic!("{style:?}: {other:?}"),
            }
            assert_eq!(g.head_commit().unwrap(), antes);
            assert!(g.is_clean().unwrap(), "{style:?}");
            assert_eq!(
                std::fs::read_to_string(root.join("a.txt")).unwrap(),
                "main\n"
            );
            assert!(!root.join("solo-x.txt").exists());
            assert!(!root.join(".git/MERGE_HEAD").exists());
        }
    }

    #[test]
    fn precondiciones() {
        let (_t, root) = con_rama();
        let g = Git::new(&root);
        identidad(&g);
        assert!(matches!(
            g.merge_branch("nada", MergeStyle::Merge, "m"),
            Err(Error::BranchNotFound(_))
        ));
        assert!(matches!(
            g.merge_branch("x", MergeStyle::Squash, " "),
            Err(Error::EmptyMessage)
        ));
        // Los archivos sin seguimiento no cuentan como árbol sucio.
        escribir(&root, "nuevo.txt", "n");
        g.merge_branch("x", MergeStyle::FastForward, "").unwrap();
        // Árbol sucio.
        git(&root, &["checkout", "-q", "-b", "y"]);
        escribir(&root, "a.txt", "sucio\n");
        let antes = g.head_commit().unwrap();
        assert!(matches!(
            g.merge_branch("x", MergeStyle::Merge, "m"),
            Err(Error::DirtyWorktree(p)) if p == root
        ));
        assert_eq!(g.head_commit().unwrap(), antes);
        // HEAD separado.
        git(&root, &["checkout", "-q", "--", "a.txt"]);
        git(&root, &["checkout", "-q", "--detach"]);
        assert!(matches!(
            g.merge_branch("x", MergeStyle::Merge, "m"),
            Err(Error::DetachedHead)
        ));
    }

    fn git_menor_que_2_38() -> bool {
        let v = Git::new(".").run_trim(["--version"]).unwrap();
        let nums: Vec<u32> = v
            .trim_start_matches("git version ")
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|s| s.parse().ok())
            .collect();
        (
            nums.first().copied().unwrap_or(0),
            nums.get(1).copied().unwrap_or(0),
        ) < (2, 38)
    }

    #[test]
    fn has_conflicts_with_no_toca_el_arbol() {
        if git_menor_que_2_38() {
            return;
        }
        let (_t, root) = con_conflicto();
        let g = Git::new(&root);
        assert!(g.has_conflicts_with("x", "main").unwrap());
        assert!(g.is_clean().unwrap());
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "main\n"
        );
        let (_t2, root2) = con_rama();
        assert!(!Git::new(&root2).has_conflicts_with("x", "main").unwrap());
        assert!(Git::new(&root2).has_conflicts_with("nada", "main").is_err());
    }

    #[test]
    fn serde_kebab_case() {
        assert_eq!(
            serde_json::to_string(&MergeStyle::FastForward).unwrap(),
            "\"fast-forward\""
        );
        assert_eq!(
            serde_json::from_str::<MergeStyle>("\"squash\"").unwrap(),
            MergeStyle::Squash
        );
    }
}
