//! Creación de commits.

use super::{failed, Git, GitOutput};
use crate::error::{Error, Result};

impl Git {
    /// Añade todos los cambios (`add -A`) y los confirma con `message`; devuelve el sha nuevo.
    ///
    /// Usa la identidad configurada del usuario. Errores: [`Error::EmptyMessage`],
    /// [`Error::NothingToCommit`] y [`Error::GitIdentityMissing`].
    pub fn commit_all(&self, message: &str) -> Result<String> {
        check_message(message)?;
        if self.is_clean()? {
            return Err(Error::NothingToCommit);
        }
        self.run(["add", "-A"])?;
        self.commit_staged(message)
    }

    /// Confirma lo que ya está en el índice con `message` (por stdin) y devuelve el sha.
    pub(super) fn commit_staged(&self, message: &str) -> Result<String> {
        let args = ["commit", "-F", "-"];
        let out = self.output_with_stdin(&args, message)?;
        if !out.success() {
            return Err(identity_error(&out).unwrap_or_else(|| failed(&args, &out)));
        }
        self.head_commit()
    }
}

pub(super) fn check_message(message: &str) -> Result<()> {
    if message.trim().is_empty() {
        Err(Error::EmptyMessage)
    } else {
        Ok(())
    }
}

/// `Some(GitIdentityMissing)` si el fallo de git se debe a la falta de identidad.
pub(super) fn identity_error(out: &GitOutput) -> Option<Error> {
    let s = &out.stderr;
    (s.contains("Author identity unknown")
        || s.contains("Please tell me who you are")
        || s.contains("unable to auto-detect email address"))
    .then_some(Error::GitIdentityMissing)
}

#[cfg(test)]
mod tests {
    use super::super::branch::testutil::*;
    use super::*;

    fn con_identidad(g: &Git) {
        g.run(["config", "user.name", "t"]).unwrap();
        g.run(["config", "user.email", "t@t"]).unwrap();
    }

    #[test]
    fn commit_all_crea_commit_con_sha_nuevo() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        con_identidad(&g);
        let antes = g.head_commit().unwrap();
        std::fs::write(root.join("b.txt"), "b").unwrap();
        std::fs::write(root.join("a.txt"), "cambio\n").unwrap();
        let sha = g.commit_all("asunto\n\ncuerpo con \"comillas\"").unwrap();
        assert_ne!(sha, antes);
        assert_eq!(sha, g.head_commit().unwrap());
        assert!(g.is_clean().unwrap());
        let msg = g.run_trim(["log", "-1", "--format=%B"]).unwrap();
        assert!(msg.starts_with("asunto"));
        assert!(msg.contains("cuerpo con \"comillas\""));
    }

    #[test]
    fn sin_cambios_y_mensaje_vacio() {
        let (_t, root) = repo();
        let g = Git::new(&root);
        assert!(matches!(g.commit_all("x"), Err(Error::NothingToCommit)));
        std::fs::write(root.join("b.txt"), "b").unwrap();
        assert!(matches!(g.commit_all("  \n"), Err(Error::EmptyMessage)));
        assert!(!g.is_clean().unwrap());
    }

    #[test]
    fn deteccion_de_identidad_faltante() {
        let mk = |s: &str| GitOutput {
            code: Some(128),
            stdout: String::new(),
            stderr: s.into(),
        };
        assert!(matches!(
            identity_error(&mk(
                "fatal: unable to auto-detect email address (got 'a@b.(none)')"
            )),
            Some(Error::GitIdentityMissing)
        ));
        assert!(matches!(
            identity_error(&mk(
                "Author identity unknown\n\n*** Please tell me who you are."
            )),
            Some(Error::GitIdentityMissing)
        ));
        assert!(identity_error(&mk("otro fallo")).is_none());
    }
}
