//! Núcleo de qorvi: tareas, configuración, persistencia y git.

pub mod config;
pub mod error;
pub mod git;
pub mod paths;
pub mod repo;

pub use config::{AgentConfig, Config, ConfigLayer, KeyStyle, KeysConfig};
pub use error::{Error, Result};
pub use git::{Git, GitOutput};
pub use paths::{Paths, RepoPaths};
pub use repo::Repo;
