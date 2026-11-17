//! Errores de `qorvi-core`.

use std::path::PathBuf;

/// Resultado con el [`Error`] de `qorvi-core`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errores del núcleo. Los mensajes van en español y están pensados para el usuario final.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// No se encontró el ejecutable `git` en el `PATH`.
    #[error("no se encontró `git`; instálalo y asegúrate de que está en el PATH")]
    GitNotFound,

    /// `git` no se pudo lanzar o su salida no se pudo leer.
    #[error("no se pudo ejecutar `git {args}`: {source}")]
    GitSpawn {
        args: String,
        #[source]
        source: std::io::Error,
    },

    /// `git` terminó con un código de salida distinto de cero.
    #[error("`git {args}` falló ({}): {stderr}", exit_text(*.code))]
    GitFailed {
        args: String,
        /// Código de salida, o `None` si el proceso terminó por una señal.
        code: Option<i32>,
        stderr: String,
    },

    /// La salida de `git` no era UTF-8 o no tenía el formato esperado.
    #[error("salida inesperada de `git {args}`: {detail}")]
    GitOutput { args: String, detail: String },

    /// La ruta no está dentro de un repositorio git.
    #[error("{} no está dentro de un repositorio git", .0.display())]
    NotARepo(PathBuf),

    /// El repositorio es *bare* (sin árbol de trabajo); qorvi necesita uno normal.
    #[error("{} es un repositorio bare; qorvi necesita un repositorio con árbol de trabajo", .0.display())]
    BareRepo(PathBuf),

    /// El nombre de tarea no es válido (también es nombre de rama y de directorio).
    #[error("nombre de tarea inválido `{name}`: {reason}")]
    InvalidTaskName { name: String, reason: String },

    /// Ya existe una tarea con ese nombre (o uno que solo difiere en mayúsculas).
    #[error("ya existe la tarea `{0}`")]
    TaskExists(String),

    /// No existe la tarea pedida.
    #[error("no existe la tarea `{0}`")]
    TaskNotFound(String),

    /// El archivo de estado no se pudo interpretar.
    #[error("estado corrupto en {}: {message}", path.display())]
    CorruptState { path: PathBuf, message: String },

    /// El archivo de estado lo escribió una versión más nueva de qorvi.
    #[error("{} usa la versión de estado {found}, pero esta versión de qorvi solo entiende hasta la {supported}; actualiza qorvi", path.display())]
    UnsupportedStateVersion {
        path: PathBuf,
        found: u64,
        supported: u64,
    },

    /// No se pudo determinar el directorio personal del usuario.
    #[error(
        "no se pudo determinar el directorio personal; define QORVI_CONFIG_DIR y QORVI_DATA_DIR"
    )]
    NoHomeDir,

    /// Archivo de configuración con TOML inválido o campos desconocidos.
    #[error("config inválida en {}: {message}", path.display())]
    Config { path: PathBuf, message: String },

    /// El agente pedido no está definido en la config.
    #[error("el agente `{name}` no existe; agentes disponibles: {available}")]
    UnknownAgent { name: String, available: String },

    /// No se indicó agente y no hay `default_agent` en la config.
    #[error(
        "no hay agente por defecto; define `default_agent` en la config. Agentes disponibles: {available}"
    )]
    NoDefaultAgent { available: String },

    /// Ya existe una rama con ese nombre.
    #[error("ya existe la rama `{0}`")]
    BranchExists(String),

    /// La ruta de destino ya existe y no está vacía.
    #[error("{} ya existe y no está vacío", .0.display())]
    PathExists(PathBuf),

    /// El worktree de la tarea tiene cambios sin commitear o archivos sin seguimiento.
    #[error("la tarea `{0}` tiene cambios sin commitear o archivos nuevos; usa --force para borrarla igualmente")]
    TaskHasChanges(String),

    /// La rama de la tarea tiene commits que no están en su rama base.
    #[error("la rama de la tarea `{task}` tiene commits que no están en `{base}`; haz merge, conserva la rama o usa --force")]
    UnmergedBranch { task: String, base: String },

    /// El worktree indicado no es un huérfano de qorvi.
    #[error("{} no es un worktree huérfano de qorvi", .0.display())]
    NotAnOrphan(PathBuf),

    /// No hay rama de partida: HEAD separado y sin rama por defecto.
    #[error("no se pudo deducir la rama de partida (HEAD separado); indícala con --base")]
    NoBaseBranch,

    /// Error de E/S sobre una ruta concreta.
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl Error {
    /// Construye un [`Error::Io`] asociado a `path`.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

fn exit_text(code: Option<i32>) -> String {
    match code {
        Some(c) => format!("código {c}"),
        None => "terminado por una señal".to_string(),
    }
}
