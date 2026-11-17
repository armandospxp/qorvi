//! Cambios de una tarea, diff por archivo y lista de archivos del worktree.
//!
//! Los cambios de una tarea son todo lo que difiere entre el merge-base de la rama base y
//! `HEAD` y el árbol de trabajo actual: commits del agente, cambios sin commitear y archivos
//! sin seguimiento. Las rutas son relativas a la raíz del worktree (se asume que `cwd` es
//! esa raíz).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use super::{failed, Git};
use crate::error::{Error, Result};

/// Tipo de cambio de un archivo respecto al merge-base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed { from: PathBuf },
    Copied { from: PathBuf },
    TypeChanged,
}

/// Un archivo cambiado. `path` es la ruta actual (la nueva en renombrados).
///
/// `additions`/`deletions` son `None` si el archivo es binario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: ChangeKind,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub binary: bool,
}

/// Tamaño del prefijo que se inspecciona para decidir si un archivo es binario (como git).
const BINARY_PROBE: usize = 8000;

impl Git {
    /// Ancestro común más reciente de `a` y `b` (sha completo).
    pub fn merge_base(&self, a: &str, b: &str) -> Result<String> {
        let a = self.resolve_commit(a)?;
        let b = self.resolve_commit(b)?;
        self.run_trim(["merge-base", &a, &b])
    }

    /// Archivos cambiados respecto al merge-base de `base` y `HEAD`, ordenados por ruta.
    /// Incluye cambios sin commitear y archivos sin seguimiento (como `Added`).
    pub fn changes_since(&self, base: &str) -> Result<Vec<FileChange>> {
        let mb = self.merge_base(base, "HEAD")?;
        let names = self.run([
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "-z",
            "-M",
            "--name-status",
            &mb,
        ])?;
        let nums = self.run([
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "-z",
            "-M",
            "--numstat",
            &mb,
        ])?;
        let stats = parse_numstat(&nums)?;
        let mut out: Vec<FileChange> = parse_name_status(&names)?
            .into_iter()
            .map(|(kind, path)| {
                let (additions, deletions) = stats.get(&path).copied().unwrap_or((None, None));
                FileChange {
                    path,
                    kind,
                    additions,
                    deletions,
                    binary: additions.is_none(),
                }
            })
            .collect();

        for path in self.untracked(None)? {
            let (additions, binary) = match count_lines(&self.cwd().join(&path)) {
                Some(n) => (Some(n), false),
                None => (None, true),
            };
            out.push(FileChange {
                path,
                kind: ChangeKind::Added,
                additions,
                deletions: if binary { None } else { Some(0) },
                binary,
            });
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// Diff unificado (sin color) de `path` contra el merge-base de `base` y `HEAD`.
    ///
    /// - Archivo sin seguimiento: `git diff --no-index` contra el archivo nulo.
    /// - Renombrado: se pasan la ruta antigua y la nueva para que git lo muestre como
    ///   renombrado y no como borrado más un alta.
    ///
    /// Devuelve cadena vacía si el archivo no tiene diferencias.
    pub fn file_diff(&self, base: &str, path: &Path) -> Result<String> {
        let mb = self.merge_base(base, "HEAD")?;
        let p = path.to_string_lossy().into_owned();
        if !self.untracked(Some(path))?.is_empty() {
            let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
            let args = [
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-color",
                "--",
                null,
                &p,
            ];
            let out = self.output(&args)?;
            // Con `--no-index`, el código 1 significa "hay diferencias".
            return match out.code {
                Some(0 | 1) => Ok(out.stdout),
                _ => Err(failed(&args, &out)),
            };
        }
        let mut args = vec![
            "diff".to_string(),
            "--no-ext-diff".into(),
            "--no-color".into(),
            "-M".into(),
            mb,
            "--".into(),
        ];
        let from = self
            .changes_since(base)?
            .into_iter()
            .find(|c| c.path == path)
            .and_then(|c| match c.kind {
                ChangeKind::Renamed { from } => Some(from),
                _ => None,
            });
        if let Some(from) = from {
            args.push(from.to_string_lossy().into_owned());
        }
        args.push(p);
        self.run(args)
    }

    /// Archivos del worktree para el explorador: seguidos más no seguidos y no ignorados,
    /// sin duplicados, ordenados, sin los seguidos que ya no existen en disco.
    pub fn list_files(&self) -> Result<Vec<PathBuf>> {
        let out = self.run([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])?;
        let set: BTreeSet<PathBuf> = split_nul(&out)
            .map(PathBuf::from)
            .filter(|p| self.cwd().join(p).symlink_metadata().is_ok())
            .collect();
        Ok(set.into_iter().collect())
    }

    /// Archivos sin seguimiento y no ignorados (opcionalmente solo `path`).
    fn untracked(&self, path: Option<&Path>) -> Result<Vec<PathBuf>> {
        let mut args = vec![
            "ls-files".to_string(),
            "--others".into(),
            "--exclude-standard".into(),
            "-z".into(),
        ];
        if let Some(p) = path {
            args.push("--".into());
            args.push(p.to_string_lossy().into_owned());
        }
        let out = self.run(args)?;
        Ok(split_nul(&out).map(PathBuf::from).collect())
    }
}

fn split_nul(s: &str) -> impl Iterator<Item = &str> {
    s.split('\0').filter(|f| !f.is_empty())
}

fn bad(args: &str, detail: String) -> Error {
    Error::GitOutput {
        args: args.into(),
        detail,
    }
}

/// Líneas de un archivo, o `None` si es binario (NUL en los primeros 8000 bytes) o ilegible.
fn count_lines(path: &Path) -> Option<u32> {
    let data = std::fs::read(path).ok()?;
    if data[..data.len().min(BINARY_PROBE)].contains(&0) {
        return None;
    }
    let newlines = data.iter().filter(|&&b| b == b'\n').count();
    let partial = usize::from(!data.is_empty() && data.last() != Some(&b'\n'));
    Some((newlines + partial) as u32)
}

const NAME_STATUS: &str = "diff -z --name-status";
const NUMSTAT: &str = "diff -z --numstat";

/// Parsea `git diff -z --name-status`: `M\0ruta\0` y `R100\0origen\0destino\0`.
pub(crate) fn parse_name_status(s: &str) -> Result<Vec<(ChangeKind, PathBuf)>> {
    let mut out = Vec::new();
    let mut fields = split_nul(s);
    while let Some(code) = fields.next() {
        let mut next = || {
            fields
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| bad(NAME_STATUS, format!("falta la ruta tras `{code}`")))
        };
        let (kind, path) = match code.as_bytes()[0] {
            b'A' => (ChangeKind::Added, next()?),
            b'M' => (ChangeKind::Modified, next()?),
            b'D' => (ChangeKind::Deleted, next()?),
            b'T' => (ChangeKind::TypeChanged, next()?),
            b'R' => {
                let from = next()?;
                (ChangeKind::Renamed { from }, next()?)
            }
            b'C' => {
                let from = next()?;
                (ChangeKind::Copied { from }, next()?)
            }
            _ => return Err(bad(NAME_STATUS, format!("código desconocido `{code}`"))),
        };
        out.push((kind, path));
    }
    Ok(out)
}

type Stat = (Option<u32>, Option<u32>);

/// Parsea `git diff -z --numstat` a un mapa ruta (la nueva) -> (adds, dels); `None` = binario.
pub(crate) fn parse_numstat(s: &str) -> Result<HashMap<PathBuf, Stat>> {
    let mut out = HashMap::new();
    let mut fields = split_nul(s);
    while let Some(entry) = fields.next() {
        let mut parts = entry.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(bad(NUMSTAT, format!("entrada inválida: {entry:?}")));
        };
        let num = |v: &str| -> Result<Option<u32>> {
            if v == "-" {
                Ok(None)
            } else {
                v.parse()
                    .map(Some)
                    .map_err(|_| bad(NUMSTAT, format!("número inválido: {entry:?}")))
            }
        };
        let stat = (num(a)?, num(d)?);
        let path = if path.is_empty() {
            // Renombrado/copia: `adds\tdels\t\0origen\0destino\0`.
            let _from = fields.next();
            fields
                .next()
                .ok_or_else(|| bad(NUMSTAT, format!("falta el destino: {entry:?}")))?
        } else {
            path
        };
        out.insert(PathBuf::from(path), stat);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::branch::testutil::*;
    use super::*;

    #[test]
    fn parse_name_status_literal() {
        let s = "M\0src/con espacios.rs\0A\0canción ñ.txt\0D\0x\0T\0l\0\
                 R100\0viejo nombre.txt\0nuevo é.txt\0C75\0a\0b\0";
        let v = parse_name_status(s).unwrap();
        assert_eq!(v.len(), 6);
        assert_eq!(v[0], (ChangeKind::Modified, "src/con espacios.rs".into()));
        assert_eq!(v[1], (ChangeKind::Added, "canción ñ.txt".into()));
        assert_eq!(v[2].0, ChangeKind::Deleted);
        assert_eq!(v[3].0, ChangeKind::TypeChanged);
        assert_eq!(
            v[4],
            (
                ChangeKind::Renamed {
                    from: "viejo nombre.txt".into()
                },
                "nuevo é.txt".into()
            )
        );
        assert_eq!(v[5].0, ChangeKind::Copied { from: "a".into() });
        assert!(parse_name_status("").unwrap().is_empty());
        assert!(parse_name_status("Z\0x\0").is_err());
        assert!(parse_name_status("M\0").is_err());
    }

    #[test]
    fn parse_numstat_literal() {
        let s = "3\t1\tsrc/con espacios.rs\0-\t-\timg.png\0\
                 0\t2\t\0viejo nombre.txt\0nuevo é.txt\0\x37\t0\tfin\0";
        let m = parse_numstat(s).unwrap();
        assert_eq!(m.len(), 4);
        assert_eq!(m[Path::new("src/con espacios.rs")], (Some(3), Some(1)));
        assert_eq!(m[Path::new("img.png")], (None, None));
        assert_eq!(m[Path::new("nuevo é.txt")], (Some(0), Some(2)));
        assert_eq!(m[Path::new("fin")], (Some(7), Some(0)));
        assert!(parse_numstat("x\ty\tz\0").is_err());
        assert!(parse_numstat("1\t2\t\0solo\0").is_err());
    }

    #[test]
    fn cuenta_lineas() {
        let dir = tempfile::tempdir().unwrap();
        let f = |name: &str, data: &[u8]| {
            let p = dir.path().join(name);
            std::fs::write(&p, data).unwrap();
            count_lines(&p)
        };
        assert_eq!(f("a", b""), Some(0));
        assert_eq!(f("b", b"x\ny\n"), Some(2));
        assert_eq!(f("c", b"x\ny"), Some(2));
        assert_eq!(f("d", b"x\0y\n"), None);
    }

    fn write(root: &Path, name: &str, data: impl AsRef<[u8]>) {
        let p = root.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    /// Rama `tarea` con commits y cambios sin commitear; `main` avanza con `solo-main.txt`.
    fn escenario() -> (tempfile::TempDir, PathBuf) {
        let (t, root) = repo();
        let largo = "línea de relleno para similitud\n".repeat(10);
        write(&root, "mod.txt", "1\n2\n3\n");
        write(&root, "borrar.txt", "x\ny\n");
        write(&root, "viejo nombre.txt", &largo);
        write(&root, "bin.dat", [1u8, 0, 2]);
        commit(&root, "base");
        git(&root, &["checkout", "-q", "-b", "tarea"]);
        write(&root, "mod.txt", "1\ndos\n3\n4\n");
        write(&root, "nuevo ñ.txt", "n1\nn2\n");
        std::fs::remove_file(root.join("borrar.txt")).unwrap();
        git(&root, &["mv", "viejo nombre.txt", "nuevo nombre.txt"]);
        write(&root, "bin.dat", [9u8, 0, 0, 0]);
        commit(&root, "tarea 1");
        git(&root, &["checkout", "-q", "main"]);
        write(&root, "solo-main.txt", "m\n");
        commit(&root, "main avanza");
        git(&root, &["checkout", "-q", "tarea"]);
        // Sin commitear + untracked.
        write(&root, "a.txt", "a\nmás\n");
        write(&root, "sin seguir.txt", "u1\nu2\nu3");
        write(&root, "sin seguir.bin", [0u8, 1, 2]);
        (t, root)
    }

    #[test]
    fn merge_base_ignora_avance_de_main() {
        let (_t, root) = escenario();
        let g = Git::new(&root);
        let mb = g.merge_base("main", "HEAD").unwrap();
        assert_eq!(mb, g.resolve_commit("main~1").unwrap());
        assert!(g.merge_base("--version", "HEAD").is_err());
    }

    #[test]
    fn cambios_de_la_tarea() {
        let (_t, root) = escenario();
        let v = Git::new(&root).changes_since("main").unwrap();
        let find = |p: &str| v.iter().find(|c| c.path == Path::new(p)).unwrap();
        let names: Vec<_> = v.iter().map(|c| c.path.to_str().unwrap()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        assert!(!names.contains(&"solo-main.txt"));

        let m = find("mod.txt");
        assert_eq!(m.kind, ChangeKind::Modified);
        assert_eq!((m.additions, m.deletions), (Some(2), Some(1)));
        assert_eq!(find("nuevo ñ.txt").kind, ChangeKind::Added);
        assert_eq!(find("borrar.txt").kind, ChangeKind::Deleted);
        assert_eq!(find("borrar.txt").deletions, Some(2));
        assert_eq!(
            find("nuevo nombre.txt").kind,
            ChangeKind::Renamed {
                from: "viejo nombre.txt".into()
            }
        );
        assert!(!names.contains(&"viejo nombre.txt"));
        assert_eq!(find("a.txt").additions, Some(1));
        let b = find("bin.dat");
        assert!(b.binary && b.additions.is_none() && b.deletions.is_none());
        let u = find("sin seguir.txt");
        assert_eq!(u.kind, ChangeKind::Added);
        assert_eq!(
            (u.additions, u.deletions, u.binary),
            (Some(3), Some(0), false)
        );
        assert!(find("sin seguir.bin").binary);
    }

    #[test]
    fn diff_de_archivos() {
        let (_t, root) = escenario();
        let g = Git::new(&root);
        let d = g.file_diff("main", Path::new("mod.txt")).unwrap();
        assert!(d.contains("-2\n") && d.contains("+dos\n") && d.contains("+4\n"));
        assert!(!d.contains('\x1b'));

        let u = g.file_diff("main", Path::new("sin seguir.txt")).unwrap();
        assert!(u.contains("+u1\n") && u.contains("+u3\n"));
        assert!(u.contains("new file mode"));

        let del = g.file_diff("main", Path::new("borrar.txt")).unwrap();
        assert!(del.contains("deleted file mode") && del.contains("-x\n"));

        let r = g.file_diff("main", Path::new("nuevo nombre.txt")).unwrap();
        assert!(r.contains("rename from viejo nombre.txt"), "{r}");

        let b = g.file_diff("main", Path::new("bin.dat")).unwrap();
        assert!(b.contains("Binary files"));
        assert!(!g.file_diff("main", Path::new("a.txt")).unwrap().is_empty());
    }

    #[test]
    fn lista_de_archivos_respeta_gitignore() {
        let (_t, root) = repo();
        write(&root, ".gitignore", "*.log\ntarget/\n");
        write(&root, "src/m.rs", "fn main() {}");
        write(&root, "borrado.txt", "b");
        commit(&root, "más");
        std::fs::remove_file(root.join("borrado.txt")).unwrap();
        write(&root, "sin seguir é.txt", "u");
        write(&root, "x.log", "l");
        write(&root, "target/o", "o");
        write(&root, "a.txt", "cambiado");
        let v = Git::new(&root).list_files().unwrap();
        let v: Vec<_> = v.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(v, [".gitignore", "a.txt", "sin seguir é.txt", "src/m.rs"]);
    }
}
