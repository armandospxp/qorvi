//! Núcleo de qorvi: tareas, configuración, persistencia y git.

pub mod error;
pub mod git;
pub mod repo;
pub mod store;
pub mod task;

pub use error::{Error, Result};
pub use git::{Git, GitOutput};
pub use repo::Repo;
pub use store::{Store, STATE_VERSION};
pub use task::{Task, TaskName};
