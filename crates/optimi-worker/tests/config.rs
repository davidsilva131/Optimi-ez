#![cfg(windows)]

use optimi_worker::{Config, ConfigError};

const PIPE: &str = "optimi-ez-worker-00112233445566778899aabbccddeeff";

fn args(pipe: &str) -> Vec<String> {
    vec!["--pipe".to_owned(), pipe.to_owned()]
}

#[test]
fn parses_the_one_time_worker_channel() {
    let config = Config::parse(args(PIPE)).expect("a valid channel parses");

    assert_eq!(config.pipe_name, PIPE);
}

#[test]
fn refuses_a_missing_value() {
    assert_eq!(
        Config::parse(vec!["--pipe".to_owned()]),
        Err(ConfigError::MissingValue)
    );
}

#[test]
fn refuses_a_missing_channel() {
    assert_eq!(Config::parse(Vec::new()), Err(ConfigError::MissingPipeName));
}

#[test]
fn refuses_an_unknown_argument() {
    let mut values = args(PIPE);
    values.push("--service".to_owned());
    values.push("DiagTrack".to_owned());

    assert_eq!(Config::parse(values), Err(ConfigError::UnknownArgument));
}

#[test]
fn refuses_a_journal_path_argument() {
    let mut values = args(PIPE);
    values.push("--journal".to_owned());
    values.push("C:/Users/player/AppData/Roaming/journal.json".to_owned());

    assert_eq!(Config::parse(values), Err(ConfigError::UnknownArgument));
}

#[test]
fn refuses_a_consenting_sid_argument() {
    let mut values = args(PIPE);
    values.push("--consenting-sid".to_owned());
    values.push("S-1-5-21-1004336348-1177238915-682003330-1001".to_owned());

    assert_eq!(Config::parse(values), Err(ConfigError::UnknownArgument));
}

#[test]
fn refuses_a_client_process_id_argument() {
    let mut values = args(PIPE);
    values.push("--client-pid".to_owned());
    values.push("4242".to_owned());

    assert_eq!(Config::parse(values), Err(ConfigError::UnknownArgument));
}

#[test]
fn refuses_a_pipe_name_outside_the_worker_namespace() {
    assert_eq!(
        Config::parse(args("\\\\.\\pipe\\anything")),
        Err(ConfigError::InvalidPipeName)
    );
}
