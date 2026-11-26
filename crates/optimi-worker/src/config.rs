use std::path::PathBuf;

use optimi_ipc::is_valid_pipe_name;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub pipe_name: String,
    pub journal: PathBuf,
    pub consenting_sid: String,
    pub client_process_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    UnknownArgument,
    MissingValue,
    MissingPipeName,
    MissingJournal,
    MissingConsentingSid,
    MissingClientProcessId,
    InvalidPipeName,
    InvalidJournalPath,
    InvalidSid,
    InvalidClientProcessId,
}

impl Config {
    #[cfg(windows)]
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, ConfigError> {
        let mut pipe_name = None;
        let mut journal = None;
        let mut consenting_sid = None;
        let mut client_process_id = None;
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let value = args.next().ok_or(ConfigError::MissingValue)?;
            match flag.as_str() {
                "--pipe" => pipe_name = Some(value),
                "--journal" => journal = Some(PathBuf::from(value)),
                "--consenting-sid" => consenting_sid = Some(value),
                "--client-pid" => client_process_id = Some(value),
                _ => return Err(ConfigError::UnknownArgument),
            }
        }
        let pipe_name = pipe_name.ok_or(ConfigError::MissingPipeName)?;
        if !is_valid_pipe_name(&pipe_name) {
            return Err(ConfigError::InvalidPipeName);
        }
        let journal = journal.ok_or(ConfigError::MissingJournal)?;
        if !journal.is_absolute() {
            return Err(ConfigError::InvalidJournalPath);
        }
        let consenting_sid = consenting_sid.ok_or(ConfigError::MissingConsentingSid)?;
        let consenting_sid = optimi_ipc::identity::canonical_sid(&consenting_sid)
            .map_err(|_| ConfigError::InvalidSid)?;
        let client_process_id = client_process_id.ok_or(ConfigError::MissingClientProcessId)?;
        let client_process_id: u32 = client_process_id
            .parse()
            .map_err(|_| ConfigError::InvalidClientProcessId)?;
        if client_process_id == 0 {
            return Err(ConfigError::InvalidClientProcessId);
        }
        Ok(Self {
            pipe_name,
            journal,
            consenting_sid,
            client_process_id,
        })
    }
}
