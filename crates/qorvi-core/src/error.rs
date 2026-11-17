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
