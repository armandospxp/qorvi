//! Núcleo de qorvi: tareas, configuración, persistencia y git.

pub mod config;
pub mod error;
pub mod gh;
pub mod git;
pub mod paths;
pub mod repo;
pub mod store;
pub mod task;
pub mod workspace;

pub use config::{AgentConfig, Config, ConfigLayer, KeyStyle, KeysConfig};
pub use error::{Error, Result};
pub use gh::{Gh, PrInfo};
pub use git::{
    ChangeKind, FileChange, FileState, FileStatus, Git, GitOutput, MergeStyle, WorktreeInfo,
};
pub use paths::{Paths, RepoPaths};
pub use repo::Repo;
pub use store::{Store, STATE_VERSION};
pub use task::{SessionCost, SessionInfo, Task, TaskName};
pub use workspace::{CreatedTask, NewTask, RemoveOptions, Workspace};
