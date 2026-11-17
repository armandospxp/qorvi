//! `Workspace`: el repositorio abierto por qorvi y sus tareas.
//!
//! Une [`Repo`], [`RepoPaths`], [`Config`] y el [`Store`] de tareas, y ofrece las
//! operaciones de alto nivel que usan la CLI y la TUI: crear y borrar tareas y detectar
//! worktrees huérfanos.
//!
//! Todas las mutaciones del estado pasan por [`Store::modify`] (lock entre procesos). Al
//! crear una tarea, el worktree se crea **con el lock tomado** y antes de guardar: si qorvi
//! muere entre ambos pasos queda un worktree sin tarea, que [`Workspace::orphan_worktrees`]
//! detecta y [`Workspace::remove_orphan`] limpia.

use std::cell::Cell;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::error::{Error, Result};
use crate::git::{Git, WorktreeInfo};
use crate::paths::{Paths, RepoPaths};
use crate::repo::Repo;
use crate::store::Store;
use crate::task::{Task, TaskName};

/// Repositorio abierto con su configuración y su estado.
#[derive(Debug, Clone)]
pub struct Workspace {
    repo: Repo,
    paths: RepoPaths,
    config: Config,
}

/// Parámetros para [`Workspace::create_task`].
#[derive(Debug, Clone, Default)]
pub struct NewTask {
    /// Nombre de la tarea, que también será el de su rama y su directorio.
    pub name: String,
    /// Rama o ref de partida. Por defecto, la rama actual del árbol principal o, si está
    /// separado, la rama por defecto del repositorio.
    pub base: Option<String>,
    /// Agente. Por defecto, `default_agent` de la config.
    pub agent: Option<String>,
}

/// Resultado de [`Workspace::create_task`].
#[derive(Debug, Clone)]
pub struct CreatedTask {
    pub task: Task,
    /// Avisos no fatales (p. ej. un archivo de `copy_files` que no se pudo copiar).
    pub warnings: Vec<String>,
}

/// Opciones de [`Workspace::remove_task`].
#[derive(Debug, Clone, Copy, Default)]
pub struct RemoveOptions {
    /// Borrar aunque el worktree tenga cambios sin commitear o la rama tenga commits que
    /// no están en la rama base.
    pub force: bool,
    /// Conservar la rama git (solo se borran el worktree y la tarea).
    pub keep_branch: bool,
}

impl Workspace {
    /// Descubre el repositorio que contiene `start` y carga su config.
    pub fn open(start: impl AsRef<Path>, paths: &Paths) -> Result<Workspace> {
        let repo = Repo::discover(start)?;
        let repo_paths = RepoPaths::new(paths, &repo);
        let config = Config::load(paths, Some(&repo_paths))?;
        Ok(Workspace::from_parts(repo, repo_paths, config))
    }

    /// Construye un workspace con piezas ya resueltas (útil en tests).
    pub fn from_parts(repo: Repo, paths: RepoPaths, config: Config) -> Workspace {
        Workspace {
            repo,
            paths,
            config,
        }
    }

    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    pub fn paths(&self) -> &RepoPaths {
        &self.paths
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Ejecutor git en el árbol principal.
    pub fn git(&self) -> Git {
        self.repo.git()
    }

    /// Tareas guardadas, en orden de creación.
    pub fn tasks(&self) -> Result<Vec<Task>> {
        Ok(Store::load(&self.paths.tasks_file)?.tasks().to_vec())
    }

    /// Busca una tarea por nombre.
    pub fn task(&self, name: &str) -> Result<Task> {
        Store::load(&self.paths.tasks_file)?
            .get(name)
            .cloned()
            .ok_or_else(|| Error::TaskNotFound(name.to_string()))
    }

    /// Crea una tarea: rama nueva desde `base`, worktree en el directorio de datos y
    /// copia de `copy_files`. No ejecuta `setup` (eso ocurre en la terminal de la tarea).
    pub fn create_task(&self, new: NewTask) -> Result<CreatedTask> {
        let name = TaskName::new(&new.name)?;
        let git = self.git();
        if !git.is_valid_branch_name(name.as_str())? {
            return Err(Error::InvalidTaskName {
                name: name.to_string(),
                reason: "git no lo acepta como nombre de rama".into(),
            });
        }
        let (agent, _) = self.config.agent(new.agent.as_deref())?;
        let agent = agent.to_string();
        let base_ref = match new.base {
            Some(b) => b,
            None => self.default_base()?,
        };
        let base_commit = git.resolve_commit(&base_ref)?;
        let worktree = self.paths.worktree_for(name.as_str());

        let task = Task {
            branch: name.to_string(),
            name,
            base_ref,
            base_commit,
            worktree,
            agent,
            created_at: now_unix(),
            session_id: None,
            title: None,
        };

        let created_worktree = Cell::new(false);
        let result = Store::modify(&self.paths.tasks_file, |store| {
            // Comprobar antes de tocar git, para no crear un worktree que luego sobre.
            if store
                .tasks()
                .iter()
                .any(|t| t.name.as_str().eq_ignore_ascii_case(task.name.as_str()))
            {
                return Err(Error::TaskExists(task.name.to_string()));
            }
            git.worktree_add(&task.worktree, &task.branch, &task.base_commit)?;
            created_worktree.set(true);
            store.insert(task.clone())
        });
        if let Err(e) = result {
            if created_worktree.get() {
                // Deshacer: el worktree y la rama recién creados no tienen nada del usuario.
                let _ = git.worktree_remove(&task.worktree, true);
                let _ = git.delete_branch(&task.branch, true);
            }
            return Err(e);
        }

        let warnings = copy_files(&self.repo.root, &task.worktree, &self.config.copy_files);
        Ok(CreatedTask { task, warnings })
    }

    /// Borra una tarea: su worktree, su rama (salvo `keep_branch`) y su entrada en el
    /// estado.
    ///
    /// Sin `force`, se niega si el worktree tiene cambios sin commitear o archivos sin
    /// seguimiento ([`Error::TaskHasChanges`]) o si la rama tiene commits que no están en
    /// la rama base ([`Error::UnmergedBranch`]), para no perder trabajo.
    pub fn remove_task(&self, name: &str, opts: RemoveOptions) -> Result<Task> {
        let git = self.git();
        Store::modify(&self.paths.tasks_file, |store| {
            let task = store
                .get(name)
                .cloned()
                .ok_or_else(|| Error::TaskNotFound(name.to_string()))?;
            let wt_exists = task.worktree.is_dir();

            if !opts.force {
                if wt_exists && !Git::new(&task.worktree).is_clean()? {
                    return Err(Error::TaskHasChanges(task.name.to_string()));
                }
                if !opts.keep_branch && self.branch_has_unmerged_work(&git, &task)? {
                    return Err(Error::UnmergedBranch {
                        task: task.name.to_string(),
                        base: task.base_ref.clone(),
                    });
                }
            }

            if wt_exists {
                // Ya se comprobó (o se forzó): `--force` evita que git se niegue por
                // archivos sin seguimiento.
                git.worktree_remove(&task.worktree, true)?;
            } else {
                git.worktree_prune()?;
            }
            if !opts.keep_branch && git.branch_exists(&task.branch)? {
                git.delete_branch(&task.branch, true)?;
            }
            store.remove(name)
        })
    }

    /// Worktrees dentro del directorio de worktrees de qorvi que no pertenecen a ninguna
    /// tarea (p. ej. si qorvi murió mientras creaba una).
    pub fn orphan_worktrees(&self) -> Result<Vec<WorktreeInfo>> {
        let tasks = self.tasks()?;
        let dir = normalize(&self.paths.worktrees_dir);
        let known: Vec<PathBuf> = tasks.iter().map(|t| normalize(&t.worktree)).collect();
        Ok(self
            .git()
            .worktree_list()?
            .into_iter()
            .filter(|w| {
                let p = normalize(&w.path);
                p.starts_with(&dir) && !known.contains(&p)
            })
            .collect())
    }

    /// Tareas cuyo worktree ya no existe en disco (borrado a mano, disco desmontado…).
    pub fn tasks_without_worktree(&self) -> Result<Vec<Task>> {
        Ok(self
            .tasks()?
            .into_iter()
            .filter(|t| !t.worktree.is_dir())
            .collect())
    }

    /// Elimina un worktree huérfano. No borra su rama, por si tiene trabajo.
    pub fn remove_orphan(&self, orphan: &WorktreeInfo) -> Result<()> {
        if !self.orphan_worktrees()?.contains(orphan) {
            return Err(Error::NotAnOrphan(orphan.path.clone()));
        }
        let git = self.git();
        if orphan.path.is_dir() {
            git.worktree_remove(&orphan.path, true)
        } else {
            git.worktree_prune()
        }
    }

    /// Rama de partida por defecto: la rama actual del árbol principal o la rama por
    /// defecto del repositorio.
    fn default_base(&self) -> Result<String> {
        let git = self.git();
        if let Some(b) = git.current_branch()? {
            return Ok(b);
        }
        git.default_branch()?.ok_or(Error::NoBaseBranch)
    }

    /// `true` si la rama de la tarea tiene commits propios que no llegaron a la base.
    fn branch_has_unmerged_work(&self, git: &Git, task: &Task) -> Result<bool> {
        if !git.branch_exists(&task.branch)? {
            return Ok(false);
        }
        let head = git.resolve_commit(&task.branch)?;
        if head == task.base_commit {
            return Ok(false);
        }
        // Si la rama base ya no existe, no hay contra qué comprobar: se considera trabajo
        // sin integrar (hace falta `force`).
        match git.resolve_commit(&task.base_ref) {
            Ok(_) => Ok(!git.is_merged_into(&task.branch, &task.base_ref)?),
            Err(_) => Ok(true),
        }
    }
}

/// Copia al worktree los archivos de `copy_files` (rutas relativas a la raíz del repo, sin
/// globs). Los que no existen se ignoran en silencio; los demás fallos son avisos.
fn copy_files(root: &Path, worktree: &Path, files: &[String]) -> Vec<String> {
    let mut warnings = Vec::new();
    for rel in files {
        let rel_path = Path::new(rel);
        let safe = !rel.is_empty()
            && rel_path
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
        if !safe {
            warnings.push(format!(
                "copy_files: se ignora `{rel}`: debe ser una ruta relativa dentro del repositorio"
            ));
            continue;
        }
        let src = root.join(rel_path);
        if !src.exists() {
            continue;
        }
        if !src.is_file() {
            warnings.push(format!(
                "copy_files: se ignora `{rel}`: solo se copian archivos"
            ));
            continue;
        }
        let dst = worktree.join(rel_path);
        let res = dst
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::copy(&src, &dst).map(|_| ()));
        if let Err(e) = res {
            warnings.push(format!("copy_files: no se pudo copiar `{rel}`: {e}"));
        }
    }
    warnings
}

/// Ruta comparable: canonicalizada si existe (resuelve `/var` → `/private/var` en macOS y
/// `C:/` frente a `\\?\C:\` en Windows); si no, la del padre más cercano que exista.
fn normalize(p: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    match (p.parent(), p.file_name()) {
        (Some(parent), Some(name)) => normalize(parent).join(name),
        _ => p.to_path_buf(),
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigLayer;
    use crate::git::testutil;

    struct Fixture {
        _tmp: tempfile::TempDir,
        _repo_tmp: tempfile::TempDir,
        root: PathBuf,
        ws: Workspace,
    }

    fn fixture(config: &str) -> Fixture {
        let (repo_tmp, root) = testutil::repo();
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::new(tmp.path().join("config"), tmp.path().join("data"));
        let repo = Repo::discover(&root).unwrap();
        let repo_paths = RepoPaths::new(&paths, &repo);
        let layer = ConfigLayer::parse(config, Path::new("test.toml")).unwrap();
        let config = Config::from_layers(None, Some(layer));
        Fixture {
            _tmp: tmp,
            _repo_tmp: repo_tmp,
            root,
            ws: Workspace::from_parts(repo, repo_paths, config),
        }
    }

    const CONFIG: &str = r#"
default_agent = "fake"
copy_files = [".env", "conf/local.toml", "no-existe", "../fuera", "dir"]
[agents.fake]
command = "fake-acp"
"#;

    fn new(name: &str) -> NewTask {
        NewTask {
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn crear_tarea() {
        let f = fixture(CONFIG);
        std::fs::write(f.root.join(".env"), "SECRETO=1").unwrap();
        std::fs::create_dir_all(f.root.join("conf")).unwrap();
        std::fs::write(f.root.join("conf/local.toml"), "x = 1").unwrap();
        std::fs::create_dir_all(f.root.join("dir")).unwrap();

        let created = f.ws.create_task(new("APP-12")).unwrap();
        let t = &created.task;
        assert_eq!(t.name.as_str(), "APP-12");
        assert_eq!(t.branch, "APP-12");
        assert_eq!(t.base_ref, "main");
        assert_eq!(t.agent, "fake");
        assert_eq!(t.base_commit, f.ws.git().head_commit().unwrap());
        assert!(t.worktree.starts_with(&f.ws.paths().worktrees_dir));
        assert!(t.worktree.join("a.txt").is_file());
        assert_eq!(
            Git::new(&t.worktree).current_branch().unwrap().as_deref(),
            Some("APP-12")
        );
        // copy_files: copiados, inexistente ignorado en silencio, inseguro y directorio avisan.
        assert_eq!(
            std::fs::read_to_string(t.worktree.join(".env")).unwrap(),
            "SECRETO=1"
        );
        assert!(t.worktree.join("conf/local.toml").is_file());
        assert_eq!(created.warnings.len(), 2, "{:?}", created.warnings);
        assert!(created.warnings[0].contains("../fuera"));
        assert!(created.warnings[1].contains("dir"));

        let tasks = f.ws.tasks().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(&tasks[0], t);
        assert!(f.ws.orphan_worktrees().unwrap().is_empty());
    }

    #[test]
    fn crear_con_base_y_agente_explicitos() {
        let f = fixture(
            "default_agent = \"a\"\n[agents.a]\ncommand = \"a\"\n[agents.b]\ncommand = \"b\"\n",
        );
        testutil::git(&f.root, &["branch", "develop"]);
        let t =
            f.ws.create_task(NewTask {
                name: "x".into(),
                base: Some("develop".into()),
                agent: Some("b".into()),
            })
            .unwrap()
            .task;
        assert_eq!(t.base_ref, "develop");
        assert_eq!(t.agent, "b");
    }

    #[test]
    fn crear_falla_sin_tocar_nada() {
        let f = fixture(CONFIG);
        f.ws.create_task(new("t1")).unwrap();
        // Duplicada (también si solo cambian las mayúsculas).
        assert!(matches!(
            f.ws.create_task(new("T1")),
            Err(Error::TaskExists(_))
        ));
        // Nombre inválido.
        assert!(matches!(
            f.ws.create_task(new("-x")),
            Err(Error::InvalidTaskName { .. })
        ));
        // Agente desconocido.
        assert!(matches!(
            f.ws.create_task(NewTask {
                name: "t2".into(),
                agent: Some("nadie".into()),
                ..Default::default()
            }),
            Err(Error::UnknownAgent { .. })
        ));
        // Base inexistente.
        assert!(f
            .ws
            .create_task(NewTask {
                name: "t3".into(),
                base: Some("no-existe".into()),
                ..Default::default()
            })
            .is_err());
        // Rama ya existente fuera de qorvi: no queda ni tarea ni worktree.
        testutil::git(&f.root, &["branch", "t4"]);
        assert!(matches!(
            f.ws.create_task(new("t4")),
            Err(Error::BranchExists(_))
        ));
        assert_eq!(f.ws.tasks().unwrap().len(), 1);
        assert_eq!(f.ws.git().worktree_list().unwrap().len(), 2);
    }

    #[test]
    fn borrar_tarea_limpia() {
        let f = fixture(CONFIG);
        let t = f.ws.create_task(new("t")).unwrap().task;
        let removed = f.ws.remove_task("t", RemoveOptions::default()).unwrap();
        assert_eq!(removed, t);
        assert!(!t.worktree.exists());
        assert!(!f.ws.git().branch_exists("t").unwrap());
        assert!(f.ws.tasks().unwrap().is_empty());
        assert!(matches!(
            f.ws.remove_task("t", RemoveOptions::default()),
            Err(Error::TaskNotFound(_))
        ));
    }

    #[test]
    fn borrar_protege_el_trabajo() {
        let f = fixture(CONFIG);
        let t = f.ws.create_task(new("t")).unwrap().task;

        // Archivo sin seguimiento → TaskHasChanges.
        std::fs::write(t.worktree.join("nuevo.txt"), "x").unwrap();
        assert!(matches!(
            f.ws.remove_task("t", RemoveOptions::default()),
            Err(Error::TaskHasChanges(_))
        ));

        // Commit sin integrar en main → UnmergedBranch.
        testutil::commit(&t.worktree, "trabajo");
        assert!(matches!(
            f.ws.remove_task("t", RemoveOptions::default()),
            Err(Error::UnmergedBranch { .. })
        ));
        assert_eq!(f.ws.tasks().unwrap().len(), 1);
        assert!(t.worktree.is_dir());

        // keep_branch: se borra el worktree, la rama (con el trabajo) se conserva.
        f.ws.remove_task(
            "t",
            RemoveOptions {
                keep_branch: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!t.worktree.exists());
        assert!(f.ws.git().branch_exists("t").unwrap());
    }

    #[test]
    fn borrar_rama_integrada_no_necesita_force() {
        let f = fixture(CONFIG);
        let t = f.ws.create_task(new("t")).unwrap().task;
        std::fs::write(t.worktree.join("b.txt"), "b").unwrap();
        testutil::commit(&t.worktree, "b");
        testutil::git(&f.root, &["merge", "-q", "--ff-only", "t"]);
        f.ws.remove_task("t", RemoveOptions::default()).unwrap();
        assert!(!f.ws.git().branch_exists("t").unwrap());
    }

    #[test]
    fn borrar_con_force() {
        let f = fixture(CONFIG);
        let t = f.ws.create_task(new("t")).unwrap().task;
        std::fs::write(t.worktree.join("a.txt"), "cambio").unwrap();
        std::fs::write(t.worktree.join("nuevo.txt"), "x").unwrap();
        f.ws.remove_task(
            "t",
            RemoveOptions {
                force: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!t.worktree.exists());
        assert!(f.ws.tasks().unwrap().is_empty());
    }

    #[test]
    fn worktree_borrado_a_mano() {
        let f = fixture(CONFIG);
        let t = f.ws.create_task(new("t")).unwrap().task;
        std::fs::remove_dir_all(&t.worktree).unwrap();
        assert_eq!(f.ws.tasks_without_worktree().unwrap(), vec![t.clone()]);
        f.ws.remove_task("t", RemoveOptions::default()).unwrap();
        assert!(f.ws.tasks().unwrap().is_empty());
        assert_eq!(f.ws.git().worktree_list().unwrap().len(), 1);
    }

    #[test]
    fn huerfanos() {
        let f = fixture(CONFIG);
        f.ws.create_task(new("t")).unwrap();
        // Worktree en el directorio de qorvi sin tarea (simula un fallo a mitad).
        let orphan_path = f.ws.paths().worktree_for("huerfano");
        f.ws.git()
            .worktree_add(&orphan_path, "huerfano", "main")
            .unwrap();
        // Worktree fuera del directorio de qorvi: no es asunto de qorvi.
        let ajeno = f._tmp.path().join("ajeno");
        f.ws.git().worktree_add(&ajeno, "ajeno", "main").unwrap();

        let orphans = f.ws.orphan_worktrees().unwrap();
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].branch.as_deref(), Some("huerfano"));

        f.ws.remove_orphan(&orphans[0]).unwrap();
        assert!(f.ws.orphan_worktrees().unwrap().is_empty());
        assert!(!orphan_path.exists());
        // La rama se conserva por si tenía trabajo.
        assert!(f.ws.git().branch_exists("huerfano").unwrap());

        // Un worktree de tarea no se puede borrar como huérfano.
        let t = f.ws.task("t").unwrap();
        let info =
            f.ws.git()
                .worktree_list()
                .unwrap()
                .into_iter()
                .find(|w| normalize(&w.path) == normalize(&t.worktree))
                .unwrap();
        assert!(matches!(
            f.ws.remove_orphan(&info),
            Err(Error::NotAnOrphan(_))
        ));
    }

    #[test]
    fn open_desde_el_worktree_de_una_tarea() {
        let f = fixture(CONFIG);
        let t = f.ws.create_task(new("t")).unwrap().task;
        let paths = Paths::new(f._tmp.path().join("config"), f._tmp.path().join("data"));
        let ws = Workspace::open(&t.worktree, &paths).unwrap();
        assert_eq!(normalize(&ws.repo().root), normalize(&f.root));
        assert_eq!(ws.paths().tasks_file, f.ws.paths().tasks_file);
    }
}
