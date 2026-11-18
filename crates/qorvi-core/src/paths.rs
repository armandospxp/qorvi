//! Rutas de configuración, datos y estado de qorvi.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::error::{Error, Result};
use crate::repo::Repo;

/// Directorios globales del usuario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl Paths {
    /// Resuelve los directorios con `directories`; `QORVI_CONFIG_DIR` y `QORVI_DATA_DIR`
    /// (si no están vacías) sobrescriben cada uno.
    pub fn from_env() -> Result<Paths> {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        Self::resolve(var("QORVI_CONFIG_DIR"), var("QORVI_DATA_DIR"))
    }

    fn resolve(
        config_override: Option<OsString>,
        data_override: Option<OsString>,
    ) -> Result<Paths> {
        let dirs = if config_override.is_some() && data_override.is_some() {
            None
        } else {
            Some(ProjectDirs::from("", "", "qorvi").ok_or(Error::NoHomeDir)?)
        };
        let pick = |over: Option<OsString>, f: fn(&ProjectDirs) -> &Path| match (over, &dirs) {
            (Some(o), _) => PathBuf::from(o),
            (None, Some(d)) => f(d).to_path_buf(),
            (None, None) => unreachable!("hay ProjectDirs si falta alguna variable"),
        };
        Ok(Paths {
            config_dir: pick(config_override, ProjectDirs::config_dir),
            data_dir: pick(data_override, ProjectDirs::data_dir),
        })
    }

    /// Construye `Paths` con directorios explícitos (útil en tests).
    pub fn new(config_dir: impl Into<PathBuf>, data_dir: impl Into<PathBuf>) -> Paths {
        Paths {
            config_dir: config_dir.into(),
            data_dir: data_dir.into(),
        }
    }

    /// Archivo de config global: `config_dir/config.toml`.
    pub fn global_config(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }
}

/// Rutas asociadas a un repositorio concreto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPaths {
    /// `<raíz>/.qorvi.toml`.
    pub repo_config: PathBuf,
    /// `<git-common-dir>/qorvi`: estado local que git no versiona.
    pub state_dir: PathBuf,
    /// `state_dir/tasks.json`.
    pub tasks_file: PathBuf,
    /// Directorio que contiene los worktrees de las tareas del repo.
    ///
    /// Los worktrees viven **fuera del repo** (en `data_dir/worktrees/<slug>-<hash>`) para
    /// que herramientas como cargo o npm no detecten el workspace del repo padre al
    /// ejecutarse dentro de ellos.
    pub worktrees_dir: PathBuf,
}

impl RepoPaths {
    pub fn new(paths: &Paths, repo: &Repo) -> RepoPaths {
        let state_dir = repo.common_dir.join("qorvi");
        let name = format!("{}-{}", slug(&repo.root), path_hash(&repo.root));
        RepoPaths {
            repo_config: repo.root.join(".qorvi.toml"),
            tasks_file: state_dir.join("tasks.json"),
            state_dir,
            worktrees_dir: paths.data_dir.join("worktrees").join(name),
        }
    }

    /// Historial del chat de la tarea `task_name` (lo escribe la interfaz).
    pub fn chat_file(&self, task_name: &str) -> PathBuf {
        self.state_dir
            .join("chats")
            .join(format!("{task_name}.json"))
    }

    /// Ruta del worktree de la tarea `task_name`.
    pub fn worktree_for(&self, task_name: &str) -> PathBuf {
        self.worktrees_dir.join(task_name)
    }
}

/// Nombre del directorio raíz saneado: solo `[A-Za-z0-9._-]`, máx. 32 caracteres.
fn slug(root: &Path) -> String {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(32)
        .collect();
    if s.is_empty() {
        "repo".to_string()
    } else {
        s
    }
}

/// 8 dígitos hex de un FNV-1a de 64 bits sobre la ruta (estable entre versiones de Rust).
fn path_hash(root: &Path) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in root.to_string_lossy().as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")[..8].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(root: &str) -> Repo {
        Repo {
            root: PathBuf::from(root),
            common_dir: PathBuf::from(root).join(".git"),
            current_worktree: PathBuf::from(root),
        }
    }

    #[test]
    fn config_global() {
        let p = Paths::new("/c", "/d");
        assert_eq!(p.global_config(), PathBuf::from("/c/config.toml"));
    }

    #[test]
    fn variables_sobrescriben() {
        let p = Paths::resolve(Some("/c".into()), Some("/d".into())).unwrap();
        assert_eq!(p, Paths::new("/c", "/d"));
    }

    #[test]
    fn rutas_del_repo() {
        let p = Paths::new("/c", "/d");
        let r = RepoPaths::new(&p, &repo("/home/u/mi proyecto"));
        assert_eq!(
            r.repo_config,
            PathBuf::from("/home/u/mi proyecto/.qorvi.toml")
        );
        assert_eq!(r.state_dir, PathBuf::from("/home/u/mi proyecto/.git/qorvi"));
        assert_eq!(
            r.tasks_file,
            PathBuf::from("/home/u/mi proyecto/.git/qorvi/tasks.json")
        );
        let dir = r
            .worktrees_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(dir.starts_with("mi-proyecto-"), "{dir}");
        assert_eq!(r.worktrees_dir.parent().unwrap(), Path::new("/d/worktrees"));
        assert_eq!(r.worktree_for("t1"), r.worktrees_dir.join("t1"));
    }

    #[test]
    fn slug_saneado() {
        assert_eq!(slug(Path::new("/a/mi repo ñ")), "mi-repo--");
        assert_eq!(slug(Path::new("/")), "repo");
        assert_eq!(slug(Path::new(&format!("/a/{}", "x".repeat(50)))).len(), 32);
        assert_eq!(slug(Path::new("/a/ok_1.2-x")), "ok_1.2-x");
    }

    #[test]
    fn hash_estable() {
        // Valores FNV-1a de 64 bits fijados: no deben cambiar nunca.
        assert_eq!(path_hash(Path::new("")), "cbf29ce4");
        assert_eq!(path_hash(Path::new("a")), "af63dc4c");
        assert_ne!(path_hash(Path::new("/x/a")), path_hash(Path::new("/y/a")));
    }
}
