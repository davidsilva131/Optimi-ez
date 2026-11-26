#![cfg(windows)]

use std::path::PathBuf;

use optimi_ipc::identity::{ClientIdentity, ProcessIdentity};
use optimi_worker::{Session, SessionPolicy};

const UI: &str = "C:/Program Files/Optimi-ez/optimi-ez.exe";
const SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";
const WORKER_STARTED: u64 = 10_000;

fn client(process_id: u32, creation_time: u64, user_sid: &str) -> ClientIdentity {
    ClientIdentity {
        process: ProcessIdentity {
            process_id,
            creation_time,
        },
        user_sid: user_sid.to_owned(),
        image: PathBuf::from(UI),
    }
}

fn policy() -> SessionPolicy {
    SessionPolicy::new(UI, WORKER_STARTED)
}

fn authorized() -> Session {
    policy()
        .authorize(client(4242, 9000, SID))
        .expect("the launching client is authorized")
}

#[test]
fn authorizes_the_client_that_launched_the_worker() {
    let session = authorized();

    assert!(session.allows(&client(4242, 9000, SID)));
}

#[test]
fn rejects_a_client_with_a_different_sid() {
    let session = authorized();

    assert!(!session.allows(&client(4242, 9000, "S-1-5-21-1-2-3-500")));
}

#[test]
fn rejects_a_recycled_process_id() {
    let session = authorized();
    let recycled = client(4242, WORKER_STARTED + 1, SID);

    assert_eq!(recycled.process.process_id, session.process_id());
    assert!(!session.allows(&recycled));
}

#[test]
fn rejects_a_client_that_started_after_the_worker() {
    assert!(
        policy()
            .authorize(client(4242, WORKER_STARTED, SID))
            .is_none()
    );
}

#[test]
fn rejects_a_client_running_another_image() {
    let mut foreign = client(4242, 9000, SID);
    foreign.image = PathBuf::from("C:/Users/player/Downloads/optimi-ez.exe");

    assert!(policy().authorize(foreign).is_none());
}

#[test]
fn compares_the_image_path_without_case_sensitivity() {
    let mut upper = client(4242, 9000, SID);
    upper.image = PathBuf::from("C:/PROGRAM FILES/OPTIMI-EZ/OPTIMI-EZ.EXE");

    assert!(policy().authorize(upper).is_some());
}

#[test]
fn a_session_only_admits_the_identity_it_was_created_from() {
    let session = authorized();
    let mut other = client(4242, 9000, SID);
    other.user_sid = "S-1-5-18".to_owned();

    assert!(!session.allows(&other));
    assert_eq!(session.user_sid(), SID);
}
