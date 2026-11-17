//! Modelo de tarea: nombre validado y metadatos persistidos.

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{Error, Result};

/// Longitud máxima de un nombre de tarea.
pub const MAX_TASK_NAME_LEN: usize = 64;

/// Nombre de tarea validado.
///
/// El nombre es a la vez el nombre de la rama git y del directorio del worktree, por eso
/// es estricto: 1–64 caracteres ASCII `[A-Za-z0-9._-]`, sin empezar por `-` ni `.`, sin
/// terminar en `.` ni `.lock`, sin `..` y distinto de `HEAD`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct TaskName(String);

impl TaskName {
    /// Valida `name` y construye el nombre.
    pub fn new(name: &str) -> Result<TaskName> {
        match check(name) {
            None => Ok(TaskName(name.to_string())),
            Some(reason) => Err(Error::InvalidTaskName {
                name: name.to_string(),
                reason: reason.to_string(),
            }),
        }
    }

    /// El nombre como `&str`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Devuelve el motivo por el que `name` es inválido, o `None` si es válido.
fn check(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("no puede estar vacío");
    }
    if name.len() > MAX_TASK_NAME_LEN {
        return Some("no puede tener más de 64 caracteres");
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Some("solo admite letras ASCII, dígitos, `.`, `_` y `-`");
    }
    if name.starts_with('-') {
        return Some("no puede empezar por `-`");
    }
    if name.starts_with('.') {
        return Some("no puede empezar por `.`");
    }
    if name.ends_with('.') {
        return Some("no puede terminar en `.`");
    }
    if name.ends_with(".lock") {
        return Some("no puede terminar en `.lock`");
    }
    if name.contains("..") {
        return Some("no puede contener `..`");
    }
    if name == "HEAD" {
        return Some("`HEAD` es un nombre reservado");
    }
    None
}

impl FromStr for TaskName {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        TaskName::new(s)
    }
}

impl fmt::Display for TaskName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for TaskName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for TaskName {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        TaskName::new(&s).map_err(serde::de::Error::custom)
    }
}

/// Una tarea: un worktree, una rama y un agente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub name: TaskName,
    /// Rama git de la tarea (hoy igual al nombre).
    pub branch: String,
    /// Rama o ref de la que partió (p. ej. `main`).
    pub base_ref: String,
    /// Commit (sha) del que partió.
    pub base_commit: String,
    /// Directorio del worktree.
    pub worktree: PathBuf,
    /// Agente asignado.
    pub agent: String,
    /// Creación, en segundos unix.
    pub created_at: u64,
    /// Sesión ACP, para reanudar.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Título de sesión que da el agente.
    #[serde(default)]
    pub title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombres_validos() {
        let largo = "a".repeat(64);
        for n in [
            "a",
            "fix-login",
            "feat_x.y",
            "v1.2.3",
            "A-b_C.9",
            &largo,
            "lock",
            "a.lockx",
        ] {
            assert!(TaskName::new(n).is_ok(), "debería ser válido: {n}");
        }
    }

    #[test]
    fn nombres_invalidos_por_regla() {
        let largo = "a".repeat(65);
        let casos: Vec<(&str, &str)> = vec![
            ("", "vacío"),
            (&largo, "64"),
            ("con espacio", "ASCII"),
            ("ñandú", "ASCII"),
            ("a/b", "ASCII"),
            ("a:b", "ASCII"),
            ("-x", "empezar por `-`"),
            (".x", "empezar por `.`"),
            ("x.", "terminar en `.`"),
            ("x.lock", ".lock"),
            ("a..b", ".."),
            ("HEAD", "HEAD"),
        ];
        for (n, razon) in casos {
            match TaskName::new(n) {
                Err(Error::InvalidTaskName { reason, .. }) => {
                    assert!(reason.contains(razon), "{n:?}: razón `{reason}`")
                }
                otro => panic!("{n:?} debería fallar, dio {otro:?}"),
            }
        }
    }

    #[test]
    fn head_en_minusculas_es_valido_y_sensible_a_mayusculas() {
        assert!(TaskName::new("head").is_ok());
        assert_ne!(TaskName::new("Fix").unwrap(), TaskName::new("fix").unwrap());
    }

    #[test]
    fn from_str_display_as_ref() {
        let n: TaskName = "mi-tarea".parse().unwrap();
        assert_eq!(n.to_string(), "mi-tarea");
        assert_eq!(n.as_ref(), "mi-tarea");
        assert!("-no".parse::<TaskName>().is_err());
    }

    #[test]
    fn serde_valida_al_deserializar() {
        let n: TaskName = serde_json::from_str("\"ok\"").unwrap();
        assert_eq!(serde_json::to_string(&n).unwrap(), "\"ok\"");
        assert!(serde_json::from_str::<TaskName>("\"..\"").is_err());
    }
}
