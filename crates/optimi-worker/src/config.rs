use optimi_ipc::is_valid_pipe_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub pipe_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    UnknownArgument,
    MissingValue,
    MissingPipeName,
    InvalidPipeName,
}

impl Config {
    #[cfg(windows)]
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, ConfigError> {
        let mut pipe_name = None;
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let value = args.next().ok_or(ConfigError::MissingValue)?;
            match flag.as_str() {
                "--pipe" => pipe_name = Some(value),
                _ => return Err(ConfigError::UnknownArgument),
            }
        }
        let pipe_name = pipe_name.ok_or(ConfigError::MissingPipeName)?;
        if !is_valid_pipe_name(&pipe_name) {
            return Err(ConfigError::InvalidPipeName);
        }
        Ok(Self { pipe_name })
    }
}
