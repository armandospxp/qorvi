//! Contención de `fs/*`: el agente solo lee y escribe dentro del worktree.

use std::io;
use std::path::{Component, Path, PathBuf};

/// Motivo de rechazo de una ruta.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Denied {
    #[error("la ruta debe ser absoluta")]
    Relative,
    #[error("la ruta contiene '..'")]
    ParentDir,
    #[error("no se pudo resolver la ruta: {0}")]
    Unresolvable(String),
    #[error("fuera del worktree ({0})")]
    Outside(PathBuf),
}

/// Resuelve `requested` (absoluta, según ACP) y exige que quede dentro de `root`.
///
/// `root` debe estar canonicalizada. Se siguen los symlinks: un enlace que apunta fuera
/// se rechaza. Si la ruta no existe, se canonicaliza el antepasado más cercano que sí
/// existe y se le añade el resto (sin `..`, así que no puede escapar).
pub fn contain(root: &Path, requested: &Path) -> Result<PathBuf, Denied> {
    if !requested.is_absolute() {
        return Err(Denied::Relative);
    }
    if requested.components().any(|c| c == Component::ParentDir) {
        return Err(Denied::ParentDir);
    }
    let mut existing = requested;
    let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
    let base = loop {
        match std::fs::canonicalize(existing) {
            Ok(p) => break p,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let (Some(parent), Some(name)) = (existing.parent(), existing.file_name()) else {
                    return Err(Denied::Unresolvable(e.to_string()));
                };
                rest.push(name);
                existing = parent;
            }
            Err(e) => return Err(Denied::Unresolvable(e.to_string())),
        }
    };
    let resolved = rest.iter().rev().fold(base, |p, n| p.join(n));
    if !resolved.starts_with(root) {
        return Err(Denied::Outside(resolved));
    }
    Ok(resolved)
}

/// Lee un archivo de texto; `line` es 1-based y `limit` limita el número de líneas.
pub(crate) async fn read_text(
    path: &Path,
    line: Option<u32>,
    limit: Option<u32>,
) -> io::Result<String> {
    let content = tokio::fs::read_to_string(path).await?;
    if line.is_none() && limit.is_none() {
        return Ok(content);
    }
    let skip = line.map_or(0, |l| l.saturating_sub(1) as usize);
    let take = limit.map_or(usize::MAX, |l| l as usize);
    Ok(content
        .split_inclusive('\n')
        .skip(skip)
        .take(take)
        .collect())
}

/// Escribe un archivo de texto creando los directorios que falten.
pub(crate) async fn write_text(path: &Path, content: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(path, content).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(tmp.path().join("secreto.txt"), "x").unwrap();
        let root = std::fs::canonicalize(&root).unwrap();
        (tmp, root)
    }

    #[test]
    fn dentro_del_worktree() {
        let (_tmp, root) = setup();
        assert_eq!(contain(&root, &root.join("src")).unwrap(), root.join("src"));
        assert_eq!(
            contain(&root, &root.join("src/nuevo.rs")).unwrap(),
            root.join("src/nuevo.rs")
        );
        // Directorios que aún no existen.
        assert_eq!(
            contain(&root, &root.join("a/b/c.txt")).unwrap(),
            root.join("a/b/c.txt")
        );
    }

    #[test]
    fn rechazos() {
        let (tmp, root) = setup();
        assert_eq!(
            contain(&root, &root.join("../secreto.txt")),
            Err(Denied::ParentDir)
        );
        assert_eq!(
            contain(&root, Path::new("relativo.txt")),
            Err(Denied::Relative)
        );
        assert!(matches!(
            contain(&root, &tmp.path().join("secreto.txt")),
            Err(Denied::Outside(_))
        ));
        assert!(matches!(
            contain(&root, &tmp.path().join("no/existe.txt")),
            Err(Denied::Outside(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_hacia_fuera() {
        let (tmp, root) = setup();
        std::os::unix::fs::symlink(tmp.path().join("secreto.txt"), root.join("enlace.txt"))
            .unwrap();
        assert!(matches!(
            contain(&root, &root.join("enlace.txt")),
            Err(Denied::Outside(_))
        ));
        std::os::unix::fs::symlink(tmp.path(), root.join("dir-enlace")).unwrap();
        assert!(matches!(
            contain(&root, &root.join("dir-enlace/nuevo/x.txt")),
            Err(Denied::Outside(_))
        ));
    }

    #[test]
    fn leer_por_lineas() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("f.txt");
        std::fs::write(&p, "uno\ndos\ntres\ncuatro").unwrap();
        rt.block_on(async {
            assert_eq!(
                read_text(&p, None, None).await.unwrap(),
                "uno\ndos\ntres\ncuatro"
            );
            assert_eq!(
                read_text(&p, Some(2), Some(2)).await.unwrap(),
                "dos\ntres\n"
            );
            assert_eq!(read_text(&p, Some(4), None).await.unwrap(), "cuatro");
            assert_eq!(read_text(&p, None, Some(1)).await.unwrap(), "uno\n");
            assert_eq!(read_text(&p, Some(9), None).await.unwrap(), "");
            write_text(&tmp.path().join("x/y/z.txt"), "hola")
                .await
                .unwrap();
        });
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("x/y/z.txt")).unwrap(),
            "hola"
        );
    }
}
