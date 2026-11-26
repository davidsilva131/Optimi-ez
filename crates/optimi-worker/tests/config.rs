#![cfg(windows)]

use std::path::PathBuf;

use optimi_worker::{Config, ConfigError};

const SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";
const PIPE: &str = "optimi-ez-worker-00112233445566778899aabbccddeeff";

fn args(pipe: &str, journal: &str, sid: &str) -> Vec<String> {
    vec![
        "--pipe".to_owned(),
        pipe.to_owned(),
        "--journal".to_owned(),
        journal.to_owned(),
        "--consenting-sid".to_owned(),
        sid.to_owned(),
        "--client-pid".to_owned(),
        "4242".to_owned(),
    ]
}

#[test]
fn parses_the_worker_channel_journal_and_consenting_identity() {
    let config = Config::parse(args(
        PIPE,
        "C:/Users/player/AppData/Roaming/journal.json",
        SID,
    ))
    .expect("valid arguments parse");

    assert_eq!(config.pipe_name, PIPE);
    assert_eq!(
        config.journal,
        PathBuf::from("C:/Users/player/AppData/Roaming/journal.json")
    );
    assert_eq!(config.consenting_sid, SID);
    assert_eq!(config.client_process_id, 4242);
}

#[test]
fn refuses_a_missing_value() {
    assert_eq!(
        Config::parse(vec!["--pipe".to_owned()]),
        Err(ConfigError::MissingValue)
    );
}

#[test]
fn refuses_an_unknown_argument() {
    let mut values = args(PIPE, "C:/journal.json", SID);
    values.push("--service".to_owned());
    values.push("DiagTrack".to_owned());

    assert_eq!(Config::parse(values), Err(ConfigError::UnknownArgument));
}

#[test]
fn refuses_a_pipe_name_outside_the_worker_namespace() {
    assert_eq!(
        Config::parse(args("\\\\.\\pipe\\anything", "C:/journal.json", SID)),
        Err(ConfigError::InvalidPipeName)
    );
}

#[test]
fn refuses_a_missing_client_process_id() {
    let mut values = args(PIPE, "C:/journal.json", SID);
    values.truncate(values.len() - 2);

    assert_eq!(
        Config::parse(values),
        Err(ConfigError::MissingClientProcessId)
    );
}

#[test]
fn refuses_a_non_numeric_client_process_id() {
    let mut values = args(PIPE, "C:/journal.json", SID);
    let last = values.len() - 1;
    values[last] = "ui".to_owned();

    assert_eq!(
        Config::parse(values),
        Err(ConfigError::InvalidClientProcessId)
    );
}

#[test]
fn refuses_a_zero_client_process_id() {
    let mut values = args(PIPE, "C:/journal.json", SID);
    let last = values.len() - 1;
    values[last] = "0".to_owned();

    assert_eq!(
        Config::parse(values),
        Err(ConfigError::InvalidClientProcessId)
    );
}

#[test]
fn refuses_a_relative_journal_path() {
    assert_eq!(
        Config::parse(args(PIPE, "journal.json", SID)),
        Err(ConfigError::InvalidJournalPath)
    );
}

#[test]
fn refuses_an_invalid_sid() {
    assert_eq!(
        Config::parse(args(PIPE, "C:/journal.json", "S-1-5-21-nope")),
        Err(ConfigError::InvalidSid)
    );
}
