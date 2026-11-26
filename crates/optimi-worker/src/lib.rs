pub mod config;
pub mod dispatch;
#[cfg(windows)]
mod pipe_server;
#[cfg(windows)]
mod run;

pub use config::{Config, ConfigError};
pub use dispatch::{WorkerCommands, dispatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerFailure {
    Pipe,
}

#[cfg(windows)]
pub use run::run;
