//! Paneles de terminal de qorvi: un programa (editor, shell…) en una pseudo-terminal con
//! su pantalla emulada, listo para dibujarse en la TUI.
//!
//! ```no_run
//! use qorvi_pty::{Pane, PaneCommand, PaneEvent};
//! let (tx, rx) = std::sync::mpsc::channel();
//! let pane = Pane::spawn(PaneCommand::new("bash", "/tmp"), move |ev| {
//!     let _ = tx.send(ev);
//! })?;
//! pane.write(b"ls\r")?;
//! while let Ok(PaneEvent::Output) = rx.recv() {
//!     println!("{}", pane.contents());
//! }
//! # Ok::<(), qorvi_pty::Error>(())
//! ```

pub mod keys;
pub mod mouse;
mod pane;
mod responder;

pub use pane::{Error, ExitStatus, Pane, PaneCommand, PaneEvent, PaneSize, Result, WheelAction};
pub use responder::Responder;
pub use vt100;
