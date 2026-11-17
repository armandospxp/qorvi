//! Persistencia de tareas en un archivo JSON con escritura atómica.
//!
//! Forma del archivo: `{ "version": 1, "tasks": [ ... ] }`, con las tareas en orden de
//! creación. Para mutar el estado usa siempre [`Store::modify`]: toma un lock de archivo
//! exclusivo, recarga desde disco, aplica el cambio y guarda, de modo que dos procesos de
//! qorvi (p. ej. la TUI y `qorvi new` en otra terminal) no se pisan. `load` + `save` sueltos
//! solo son seguros si ningún otro proceso escribe a la vez.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::task::Task;

/// Versión del formato de estado que entiende esta versión de qorvi.
pub const STATE_VERSION: u64 = 1;

#[derive(Serialize)]
struct FileOut<'a> {
    version: u64,
    tasks: &'a [Task],
}

#[derive(Deserialize)]
struct FileIn {
    #[serde(default)]
    tasks: Vec<Task>,
}

/// Conjunto de tareas respaldado por un archivo.
#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
    tasks: Vec<Task>,
}

impl Store {
    /// Carga el estado de `path`. Si el archivo no existe, devuelve un store vacío.
    pub fn load(path: impl AsRef<Path>) -> Result<Store> {
        let path = path.as_ref().to_path_buf();
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Store {
                    path,
                    tasks: Vec::new(),
                })
            }
            Err(e) => return Err(Error::io(&path, e)),
        };
        let corrupt = |message: String| Error::CorruptState {
            path: path.clone(),
            message,
        };
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| corrupt(e.to_string()))?;
        // La versión se mira antes de interpretar las tareas: un formato futuro puede
        // tener otra forma y no debe reportarse como corrupto.
        let version = value
            .get("version")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| corrupt("falta el campo `version` o no es un entero".into()))?;
        if version > STATE_VERSION {
            return Err(Error::UnsupportedStateVersion {
                path,
                found: version,
                supported: STATE_VERSION,
            });
        }
        let parsed: FileIn = serde_json::from_value(value).map_err(|e| corrupt(e.to_string()))?;
        Ok(Store {
            path,
            tasks: parsed.tasks,
        })
    }

    /// Ruta del archivo de estado.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Tareas en orden de creación.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// Busca una tarea por nombre (sensible a mayúsculas).
    pub fn get(&self, name: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.name.as_str() == name)
    }

    /// Añade una tarea al final. Falla si ya existe una con el mismo nombre o con uno que
    /// solo difiere en mayúsculas (macOS y Windows tienen sistemas de archivos insensibles).
    pub fn insert(&mut self, task: Task) -> Result<()> {
        let nombre = task.name.as_str();
        if self
            .tasks
            .iter()
            .any(|t| t.name.as_str().eq_ignore_ascii_case(nombre))
        {
            return Err(Error::TaskExists(nombre.to_string()));
        }
        self.tasks.push(task);
        Ok(())
    }

    /// Quita y devuelve la tarea `name`.
    pub fn remove(&mut self, name: &str) -> Result<Task> {
        let i = self
            .tasks
            .iter()
            .position(|t| t.name.as_str() == name)
            .ok_or_else(|| Error::TaskNotFound(name.to_string()))?;
        Ok(self.tasks.remove(i))
    }

    /// Modifica la tarea `name`. El closure no puede cambiar el nombre: si lo hace, la
    /// tarea se restaura y se devuelve un error.
    pub fn update(&mut self, name: &str, f: impl FnOnce(&mut Task)) -> Result<()> {
        let task = self
            .tasks
            .iter_mut()
            .find(|t| t.name.as_str() == name)
            .ok_or_else(|| Error::TaskNotFound(name.to_string()))?;
        let original = task.clone();
        f(task);
        if task.name != original.name {
            let intento = task.name.to_string();
            *task = original;
            return Err(Error::InvalidTaskName {
                name: intento,
                reason: "el nombre de una tarea no se puede cambiar".to_string(),
            });
        }
        Ok(())
    }

    /// Guarda de forma atómica: escribe a un temporal en el mismo directorio, `sync_all` y
    /// renombra sobre el destino. Nunca queda un archivo a medias.
    pub fn save(&self) -> Result<()> {
        let dir = match self.path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        let json = serde_json::to_string_pretty(&FileOut {
            version: STATE_VERSION,
            tasks: &self.tasks,
        })
        .map_err(|e| Error::CorruptState {
            path: self.path.clone(),
            message: e.to_string(),
        })?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir).map_err(|e| Error::io(dir, e))?;
        let tmp_path = tmp.path().to_path_buf();
        tmp.write_all(json.as_bytes())
            .and_then(|_| tmp.write_all(b"\n"))
            .and_then(|_| tmp.as_file().sync_all())
            .map_err(|e| Error::io(&tmp_path, e))?;
        tmp.persist(&self.path)
            .map_err(|e| Error::io(&self.path, e.error))?;
        #[cfg(unix)]
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all(); // best effort
        }
        Ok(())
    }

    /// Camino recomendado para mutar el estado: toma un lock exclusivo sobre
    /// `<path>.lock`, recarga desde disco, aplica `f`, guarda (solo si `f` devolvió `Ok`) y
    /// suelta el lock. El archivo `.lock` se deja en su sitio a propósito.
    pub fn modify<T>(path: impl AsRef<Path>, f: impl FnOnce(&mut Store) -> Result<T>) -> Result<T> {
        let path = path.as_ref();
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        let mut lock_name = path.as_os_str().to_owned();
        lock_name.push(".lock");
        let lock_path = PathBuf::from(lock_name);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|e| Error::io(&lock_path, e))?;
        lock.lock().map_err(|e| Error::io(&lock_path, e))?;
        // El lock se suelta al cerrar `lock` (fin de la función, también en errores).
        let mut store = Store::load(path)?;
        let out = f(&mut store)?;
        store.save()?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::TaskName;

    fn tarea(n: &str) -> Task {
        Task {
            name: TaskName::new(n).unwrap(),
            branch: n.to_string(),
            base_ref: "main".into(),
            base_commit: "abc123".into(),
            worktree: PathBuf::from(format!("/tmp/wt/{n}")),
            agent: "claude".into(),
            created_at: 1_700_000_000,
            session_id: None,
            title: None,
        }
    }

    #[test]
    fn roundtrip_guardar_cargar() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("sub/estado.json");
        let mut s = Store::load(&p).unwrap();
        s.insert(tarea("uno")).unwrap();
        let mut dos = tarea("dos");
        dos.session_id = Some("s-1".into());
        dos.title = Some("Título".into());
        s.insert(dos).unwrap();
        s.save().unwrap();
        let c = Store::load(&p).unwrap();
        assert_eq!(c.tasks(), s.tasks());
        assert_eq!(c.tasks()[0].name.as_str(), "uno");
        assert_eq!(c.get("dos").unwrap().session_id.as_deref(), Some("s-1"));
        assert!(fs::read_to_string(&p).unwrap().contains("\"version\": 1"));
    }

    #[test]
    fn archivo_inexistente_da_store_vacio() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::load(d.path().join("no.json")).unwrap();
        assert!(s.tasks().is_empty());
    }

    #[test]
    fn json_corrupto() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("e.json");
        fs::write(&p, "{ no es json").unwrap();
        assert!(matches!(Store::load(&p), Err(Error::CorruptState { .. })));
        fs::write(&p, "{\"tasks\": []}").unwrap();
        assert!(matches!(Store::load(&p), Err(Error::CorruptState { .. })));
        fs::write(&p, "{\"version\":1,\"tasks\":[{\"name\":\"-mal\"}]}").unwrap();
        assert!(matches!(Store::load(&p), Err(Error::CorruptState { .. })));
    }

    #[test]
    fn version_futura_no_toca_el_archivo() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("e.json");
        let texto = "{\"version\": 99, \"tasks\": \"otra forma\"}";
        fs::write(&p, texto).unwrap();
        match Store::load(&p) {
            Err(Error::UnsupportedStateVersion {
                found, supported, ..
            }) => {
                assert_eq!((found, supported), (99, STATE_VERSION));
            }
            otro => panic!("esperaba UnsupportedStateVersion, dio {otro:?}"),
        }
        assert_eq!(fs::read_to_string(&p).unwrap(), texto);
    }

    #[test]
    fn choque_de_mayusculas_y_duplicados() {
        let mut s = Store::load("/nonexistent/x.json").unwrap();
        s.insert(tarea("Fix")).unwrap();
        assert!(matches!(s.insert(tarea("Fix")), Err(Error::TaskExists(_))));
        assert!(matches!(s.insert(tarea("fix")), Err(Error::TaskExists(_))));
        assert!(s.get("fix").is_none());
        assert!(s.insert(tarea("otra")).is_ok());
    }

    #[test]
    fn remove_y_not_found() {
        let mut s = Store::load("/nonexistent/x.json").unwrap();
        s.insert(tarea("a")).unwrap();
        assert_eq!(s.remove("a").unwrap().name.as_str(), "a");
        assert!(matches!(s.remove("a"), Err(Error::TaskNotFound(_))));
        assert!(matches!(s.update("a", |_| {}), Err(Error::TaskNotFound(_))));
    }

    #[test]
    fn update_modifica_pero_no_renombra() {
        let mut s = Store::load("/nonexistent/x.json").unwrap();
        s.insert(tarea("a")).unwrap();
        s.update("a", |t| t.title = Some("hola".into())).unwrap();
        assert_eq!(s.get("a").unwrap().title.as_deref(), Some("hola"));
        let r = s.update("a", |t| {
            t.name = TaskName::new("b").unwrap();
            t.agent = "otro".into();
        });
        assert!(matches!(r, Err(Error::InvalidTaskName { .. })));
        assert_eq!(s.get("a").unwrap().agent, "claude");
        assert!(s.get("b").is_none());
    }

    #[test]
    fn opcionales_ausentes_en_json_viejo() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("e.json");
        fs::write(
            &p,
            r#"{"version":1,"tasks":[{"name":"vieja","branch":"vieja","base_ref":"main",
            "base_commit":"abc","worktree":"/w","agent":"claude","created_at":5}]}"#,
        )
        .unwrap();
        let s = Store::load(&p).unwrap();
        let t = s.get("vieja").unwrap();
        assert_eq!((t.session_id.clone(), t.title.clone()), (None, None));
    }

    #[test]
    fn modify_desde_varios_hilos() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("estado.json");
        let hilos: Vec<_> = (0..8)
            .map(|i| {
                let p = p.clone();
                std::thread::spawn(move || {
                    Store::modify(&p, |s| s.insert(tarea(&format!("t{i}")))).unwrap();
                })
            })
            .collect();
        for h in hilos {
            h.join().unwrap();
        }
        let s = Store::load(&p).unwrap();
        assert_eq!(s.tasks().len(), 8);
        for i in 0..8 {
            assert!(s.get(&format!("t{i}")).is_some());
        }
    }

    #[test]
    fn modify_con_error_no_guarda() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("estado.json");
        let r: Result<()> = Store::modify(&p, |s| {
            s.insert(tarea("a"))?;
            Err(Error::TaskNotFound("x".into()))
        });
        assert!(r.is_err());
        assert!(!p.exists());
    }

    #[test]
    fn save_no_deja_temporales() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("estado.json");
        let mut s = Store::load(&p).unwrap();
        s.insert(tarea("a")).unwrap();
        s.save().unwrap();
        s.save().unwrap();
        let nombres: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(nombres, vec!["estado.json".to_string()]);
    }
}
