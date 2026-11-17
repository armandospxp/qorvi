//! Estado del árbol de trabajo (`git status --porcelain=v2 -z`).

use std::path::PathBuf;

use super::Git;
use crate::error::{Error, Result};

/// Tipo de cambio de un archivo en el índice o en el árbol de trabajo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    Modified,
    Added,
    Deleted,
    Renamed { from: PathBuf },
    Copied { from: PathBuf },
    TypeChanged,
    Untracked,
    Conflicted,
}

/// Estado de un archivo. `path` es relativo a la raíz del árbol de trabajo.
///
/// - `staged`: cambio ya en el índice (columna X); `unstaged`: cambio aún sin añadir (Y).
/// - Archivos sin seguimiento: `staged = None`, `unstaged = Some(Untracked)`.
/// - Archivos en conflicto: `staged = None`, `unstaged = Some(Conflicted)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub path: PathBuf,
    pub staged: Option<FileState>,
    pub unstaged: Option<FileState>,
}

impl Git {
    /// Estado de todos los archivos con cambios, incluidos los untracked uno a uno.
    pub fn status(&self) -> Result<Vec<FileStatus>> {
        let out = self.run(["status", "--porcelain=v2", "-z", "--untracked-files=all"])?;
        parse_status_v2(&out)
    }

    /// `true` si no hay cambios ni archivos sin seguimiento.
    pub fn is_clean(&self) -> Result<bool> {
        Ok(self.status()?.is_empty())
    }

    /// Commits `(adelantados, atrasados)` de `branch` respecto a `upstream`.
    pub fn ahead_behind(&self, branch: &str, upstream: &str) -> Result<(u32, u32)> {
        let a = self.resolve_commit(branch)?;
        let b = self.resolve_commit(upstream)?;
        let range = format!("{a}...{b}");
        let args = ["rev-list", "--left-right", "--count", &range];
        let out = self.run_trim(args)?;
        let bad = || Error::GitOutput {
            args: args.join(" "),
            detail: format!("salida inesperada: {out:?}"),
        };
        let (l, r) = out.split_once('\t').ok_or_else(bad)?;
        Ok((
            l.trim().parse().map_err(|_| bad())?,
            r.trim().parse().map_err(|_| bad())?,
        ))
    }
}

fn bad(detail: String) -> Error {
    Error::GitOutput {
        args: "status --porcelain=v2 -z".into(),
        detail,
    }
}

fn state(c: char, from: Option<&str>) -> Result<Option<FileState>> {
    let from = || PathBuf::from(from.unwrap_or_default());
    Ok(match c {
        '.' => None,
        'M' => Some(FileState::Modified),
        'A' => Some(FileState::Added),
        'D' => Some(FileState::Deleted),
        'T' => Some(FileState::TypeChanged),
        'R' => Some(FileState::Renamed { from: from() }),
        'C' => Some(FileState::Copied { from: from() }),
        other => return Err(bad(format!("código de estado desconocido `{other}`"))),
    })
}

/// Parsea la salida de `git status --porcelain=v2 -z`. Ignora cabeceras (`#`) e
/// ignorados (`!`).
pub(crate) fn parse_status_v2(s: &str) -> Result<Vec<FileStatus>> {
    let mut out = Vec::new();
    let mut fields = s.split('\0').filter(|f| !f.is_empty());
    while let Some(entry) = fields.next() {
        let kind = entry.as_bytes()[0];
        match kind {
            b'1' | b'2' => {
                let n = if kind == b'1' { 9 } else { 10 };
                let parts: Vec<&str> = entry.splitn(n, ' ').collect();
                if parts.len() != n {
                    return Err(bad(format!("entrada incompleta: {entry:?}")));
                }
                // En las entradas `2` la ruta de origen es el siguiente campo NUL.
                let orig = if kind == b'2' { fields.next() } else { None };
                let mut xy = parts[1].chars();
                let (Some(x), Some(y)) = (xy.next(), xy.next()) else {
                    return Err(bad(format!("XY inválido: {entry:?}")));
                };
                out.push(FileStatus {
                    path: PathBuf::from(parts[n - 1]),
                    staged: state(x, orig)?,
                    unstaged: state(y, orig)?,
                });
            }
            b'u' => {
                let parts: Vec<&str> = entry.splitn(11, ' ').collect();
                if parts.len() != 11 {
                    return Err(bad(format!("entrada incompleta: {entry:?}")));
                }
                out.push(FileStatus {
                    path: PathBuf::from(parts[10]),
                    staged: None,
                    unstaged: Some(FileState::Conflicted),
                });
            }
            b'?' => {
                let path = entry
                    .strip_prefix("? ")
                    .ok_or_else(|| bad(format!("entrada inválida: {entry:?}")))?;
                out.push(FileStatus {
                    path: PathBuf::from(path),
                    staged: None,
                    unstaged: Some(FileState::Untracked),
                });
            }
            b'!' | b'#' => {}
            _ => return Err(bad(format!("tipo de entrada desconocido: {entry:?}"))),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::branch::testutil::*;
    use super::*;

    const H: &str = "1111111111111111111111111111111111111111";

    #[test]
    fn parse_literal() {
        let s = format!(
            "# branch.oid {H}\0\
             1 .M N... 100644 100644 100644 {H} {H} src/con espacios.rs\0\
             1 A. N... 000000 100644 100644 {H} {H} canción ñ.txt\0\
             1 MD N... 100644 100644 000000 {H} {H} x\0\
             2 R. N... 100644 100644 100644 {H} {H} R100 nuevo nombre.txt\0viejo nombre.txt\0\
             u UU N... 100644 100644 100644 100644 {H} {H} {H} choque.txt\0\
             ? sin seguir/é.txt\0\
             ! target/x\0"
        );
        let v = parse_status_v2(&s).unwrap();
        assert_eq!(v.len(), 6);
        assert_eq!(v[0].path, PathBuf::from("src/con espacios.rs"));
        assert_eq!(v[0].staged, None);
        assert_eq!(v[0].unstaged, Some(FileState::Modified));
        assert_eq!(v[1].path, PathBuf::from("canción ñ.txt"));
        assert_eq!(v[1].staged, Some(FileState::Added));
        assert_eq!(v[2].staged, Some(FileState::Modified));
        assert_eq!(v[2].unstaged, Some(FileState::Deleted));
        assert_eq!(v[3].path, PathBuf::from("nuevo nombre.txt"));
        assert_eq!(
            v[3].staged,
            Some(FileState::Renamed {
                from: PathBuf::from("viejo nombre.txt")
            })
        );
        assert_eq!(v[4].unstaged, Some(FileState::Conflicted));
        assert_eq!(v[5].path, PathBuf::from("sin seguir/é.txt"));
        assert_eq!(v[5].unstaged, Some(FileState::Untracked));
        assert!(parse_status_v2("").unwrap().is_empty());
        assert!(parse_status_v2("Z foo\0").is_err());
    }

    #[test]
    fn limpio_y_sucio() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        assert!(g.is_clean().unwrap());
        std::fs::write(root.join("n.txt"), "n").unwrap();
        assert!(!g.is_clean().unwrap());
    }

    #[test]
    fn estados_reales() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        std::fs::write(
            root.join("b.txt"),
            "contenido largo para similitud\n".repeat(5),
        )
        .unwrap();
        std::fs::write(root.join("c.txt"), "c").unwrap();
        commit(&root, "más");
        std::fs::write(root.join("a.txt"), "cambio\n").unwrap();
        std::fs::remove_file(root.join("c.txt")).unwrap();
        git(&root, &["mv", "b.txt", "renombrado con espacio.txt"]);
        std::fs::write(root.join("nuevo.txt"), "n").unwrap();
        git(&root, &["add", "nuevo.txt"]);
        std::fs::create_dir(root.join("d")).unwrap();
        std::fs::write(root.join("d").join("u.txt"), "u").unwrap();

        let v = g.status().unwrap();
        let get = |p: &str| {
            v.iter()
                .find(|f| f.path.as_path() == std::path::Path::new(p))
                .unwrap()
                .clone()
        };
        assert_eq!(get("a.txt").unstaged, Some(FileState::Modified));
        assert_eq!(get("c.txt").unstaged, Some(FileState::Deleted));
        assert_eq!(get("nuevo.txt").staged, Some(FileState::Added));
        assert_eq!(
            get("renombrado con espacio.txt").staged,
            Some(FileState::Renamed {
                from: PathBuf::from("b.txt")
            })
        );
        assert_eq!(get("d/u.txt").unstaged, Some(FileState::Untracked));
        assert_eq!(v.len(), 5);
    }

    #[test]
    fn conflicto_real() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        git(&root, &["checkout", "-q", "-b", "otra"]);
        std::fs::write(root.join("a.txt"), "otra\n").unwrap();
        commit(&root, "otra");
        git(&root, &["checkout", "-q", "main"]);
        std::fs::write(root.join("a.txt"), "main\n").unwrap();
        commit(&root, "main");
        let out = g
            .output(&["-c", "user.name=t", "-c", "user.email=t@t", "merge", "otra"])
            .unwrap();
        assert!(!out.success());
        let v = g.status().unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].path, PathBuf::from("a.txt"));
        assert_eq!(v[0].unstaged, Some(FileState::Conflicted));
        assert_eq!(v[0].staged, None);
    }

    #[test]
    fn adelante_atras() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        git(&root, &["branch", "base"]);
        for i in 0..2 {
            std::fs::write(root.join(format!("m{i}")), "m").unwrap();
            commit(&root, "m");
        }
        git(&root, &["checkout", "-q", "base"]);
        std::fs::write(root.join("x"), "x").unwrap();
        commit(&root, "x");
        assert_eq!(g.ahead_behind("main", "base").unwrap(), (2, 1));
        assert_eq!(g.ahead_behind("base", "main").unwrap(), (1, 2));
        assert_eq!(g.ahead_behind("main", "main").unwrap(), (0, 0));
    }
}
