pub mod config;
pub mod dispatch;
#[cfg(windows)]
mod pipe_server;
#[cfg(windows)]
mod run;
#[cfg(windows)]
pub mod session;

pub use config::{Config, ConfigError};
pub use dispatch::{WorkerCommands, dispatch};
#[cfg(windows)]
pub use session::{Session, SessionPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerFailure {
    Pipe,
    Journal,
    Identity,
}

#[cfg(windows)]
pub use run::run;
