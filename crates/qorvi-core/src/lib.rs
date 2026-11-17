//! Núcleo de qorvi: tareas, configuración, persistencia y git.

pub mod error;
pub mod git;
pub mod repo;

pub use error::{Error, Result};
pub use git::{Git, GitOutput};
pub use repo::Repo;
