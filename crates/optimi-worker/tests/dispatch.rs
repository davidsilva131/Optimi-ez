use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use optimi_domain::command::{
    AbandonedTransaction, CommandOutcome, Consent, RecoveryStatus, Refusal, RestoreReport,
};
use optimi_domain::id::{PreviewId, ProfileId, TransactionId};
use optimi_domain::model::{Preview, VerifiedChange};
use optimi_ipc::{
    Command, MAX_FRAME_BYTES, PROTOCOL_VERSION, Request, ResponseBody, decode_response,
    encode_request,
};
use optimi_worker::{WorkerCommands, dispatch};

#[derive(Clone, Default)]
struct Calls(Arc<Mutex<Vec<&'static str>>>);

impl Calls {
    fn read(&self) -> MutexGuard<'_, Vec<&'static str>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct RecordingCommands {
    calls: Calls,
}

impl RecordingCommands {
    fn new() -> (Self, Calls) {
        let calls = Calls::default();
        (
            Self {
                calls: calls.clone(),
            },
            calls,
        )
    }

    fn record(&self, call: &'static str) {
        self.calls.read().push(call);
    }
}

impl WorkerCommands for RecordingCommands {
    fn preview(&self, _: &ProfileId) -> CommandOutcome<Preview> {
        self.record("preview");
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }

    fn apply(&self, _: &PreviewId, _: &Consent) -> CommandOutcome<VerifiedChange> {
        self.record("apply");
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }

    fn undo(&self, _: &TransactionId) -> CommandOutcome<VerifiedChange> {
        self.record("undo");
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }

    fn restore_transaction(
        &self,
        _: &TransactionId,
        _: Option<u32>,
    ) -> CommandOutcome<VerifiedChange> {
        self.record("restore_transaction");
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }

    fn restore_all(&self) -> CommandOutcome<RestoreReport> {
        self.record("restore_all");
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }

    fn abandon(&self, _: &TransactionId, _: &str) -> CommandOutcome<AbandonedTransaction> {
        self.record("abandon");
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }

    fn recovery_status(&self) -> CommandOutcome<RecoveryStatus> {
        self.record("recovery_status");
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }
}

struct HugeReport {
    restored: usize,
}

impl HugeReport {
    fn with_entries(restored: usize) -> Self {
        Self { restored }
    }
}

impl WorkerCommands for HugeReport {
    fn preview(&self, _: &ProfileId) -> CommandOutcome<Preview> {
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }

    fn apply(&self, _: &PreviewId, _: &Consent) -> CommandOutcome<VerifiedChange> {
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }

    fn undo(&self, _: &TransactionId) -> CommandOutcome<VerifiedChange> {
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }

    fn restore_transaction(
        &self,
        _: &TransactionId,
        _: Option<u32>,
    ) -> CommandOutcome<VerifiedChange> {
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }

    fn restore_all(&self) -> CommandOutcome<RestoreReport> {
        let restored = (0..self.restored)
            .map(|index| VerifiedChange {
                transaction_id: TransactionId::parse(format!(
                    "tx-{index:04}-with-a-long-recorded-id"
                ))
                .expect("transaction id is valid"),
                action_id: optimi_domain::id::ActionId::parse("diagtrack-privacy")
                    .expect("action id is valid"),
                service: "DiagTrack".to_owned(),
                verified_state: optimi_domain::model::ServiceState {
                    run_state: optimi_domain::model::ServiceRunState::Running,
                    start_type: optimi_domain::model::ServiceStartType::Automatic,
                    delayed_start: true,
                },
            })
            .collect();
        CommandOutcome::Completed(RestoreReport {
            restored,
            blocked: Vec::new(),
        })
    }

    fn abandon(&self, _: &TransactionId, _: &str) -> CommandOutcome<AbandonedTransaction> {
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }

    fn recovery_status(&self) -> CommandOutcome<RecoveryStatus> {
        CommandOutcome::Refused(Refusal::WriteInProgress)
    }
}

fn frame(request_id: u64, command: Command) -> Vec<u8> {
    encode_request(&Request {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        command,
    })
    .expect("request frame encodes")
}

fn raw_frame(payload: &str) -> Vec<u8> {
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload.as_bytes());
    frame
}

fn rejected(response: &[u8]) -> optimi_ipc::ProtocolError {
    let response = decode_response(response).expect("rejection frame decodes");
    match response.body {
        ResponseBody::Rejected(error) => error,
        other => panic!("expected a rejection, found {other:?}"),
    }
}

fn preview_frame() -> Vec<u8> {
    frame(
        8,
        Command::Preview(optimi_ipc::PreviewRequest {
            profile_id: ProfileId::parse("competitive").expect("profile id is valid"),
        }),
    )
}

#[test]
fn a_valid_command_reaches_its_handler_and_keeps_the_request_id() {
    let (commands, calls) = RecordingCommands::new();

    let response = dispatch(&commands, &preview_frame());

    assert_eq!(*calls.read(), vec!["preview"]);
    let response = decode_response(&response).expect("response decodes");
    assert_eq!(response.request_id, 8);
    assert_eq!(response.protocol_version, PROTOCOL_VERSION);
    assert!(matches!(response.body, ResponseBody::Preview(_)));
}

#[test]
fn an_unknown_command_is_rejected_before_dispatch() {
    let (commands, calls) = RecordingCommands::new();
    let frame = raw_frame(
        r#"{"protocol_version":1,"request_id":3,"command":{"name":"shell","params":{"command":"whoami"}}}"#,
    );

    let response = dispatch(&commands, &frame);

    assert!(calls.read().is_empty());
    assert_eq!(rejected(&response), optimi_ipc::ProtocolError::Malformed);
}

#[test]
fn a_foreign_protocol_version_is_rejected_before_dispatch() {
    let (commands, calls) = RecordingCommands::new();
    let frame =
        raw_frame(r#"{"protocol_version":9,"request_id":3,"command":{"name":"recovery_status"}}"#);

    let response = dispatch(&commands, &frame);

    assert!(calls.read().is_empty());
    assert_eq!(
        rejected(&response),
        optimi_ipc::ProtocolError::UnsupportedVersion {
            found: 9,
            supported: PROTOCOL_VERSION
        }
    );
}

#[test]
fn an_oversized_frame_is_rejected_before_dispatch() {
    let (commands, calls) = RecordingCommands::new();
    let mut frame = Vec::new();
    frame.extend_from_slice(&((MAX_FRAME_BYTES + 1) as u32).to_le_bytes());

    let response = dispatch(&commands, &frame);

    assert!(calls.read().is_empty());
    assert_eq!(
        rejected(&response),
        optimi_ipc::ProtocolError::FrameTooLarge {
            limit: MAX_FRAME_BYTES
        }
    );
}

#[test]
fn malformed_json_is_rejected_before_dispatch() {
    let (commands, calls) = RecordingCommands::new();

    let response = dispatch(&commands, &raw_frame("not json"));

    assert!(calls.read().is_empty());
    assert_eq!(rejected(&response), optimi_ipc::ProtocolError::Malformed);
}

#[test]
fn an_unknown_field_is_rejected_before_dispatch() {
    let (commands, calls) = RecordingCommands::new();
    let frame = raw_frame(
        r#"{"protocol_version":1,"request_id":3,"command":{"name":"preview","params":{"profile_id":"competitive","registry_path":"HKLM\\System"}}}"#,
    );

    let response = dispatch(&commands, &frame);

    assert!(calls.read().is_empty());
    assert_eq!(rejected(&response), optimi_ipc::ProtocolError::Malformed);
}

#[test]
fn an_invalid_catalog_identifier_is_rejected_before_dispatch() {
    let (commands, calls) = RecordingCommands::new();
    let frame = raw_frame(
        r#"{"protocol_version":1,"request_id":3,"command":{"name":"preview","params":{"profile_id":"../admin"}}}"#,
    );

    let response = dispatch(&commands, &frame);

    assert!(calls.read().is_empty());
    assert_eq!(rejected(&response), optimi_ipc::ProtocolError::Malformed);
}

#[test]
fn every_approved_domain_command_reaches_exactly_its_handler() {
    let (commands, calls) = RecordingCommands::new();
    let preview_id = PreviewId::parse("preview-1").expect("preview id is valid");
    let transaction_id = TransactionId::parse("tx-1").expect("transaction id is valid");

    for command in [
        Command::Preview(optimi_ipc::PreviewRequest {
            profile_id: ProfileId::parse("competitive").expect("profile id is valid"),
        }),
        Command::Apply(optimi_ipc::ApplyRequest {
            preview_id: preview_id.clone(),
            consent: Consent { preview_id },
        }),
        Command::Undo(optimi_ipc::UndoRequest {
            transaction_id: transaction_id.clone(),
        }),
        Command::RestoreTransaction(optimi_ipc::RestoreTransactionRequest {
            transaction_id: transaction_id.clone(),
            confirmed_build: None,
        }),
        Command::RestoreAll,
        Command::Abandon(optimi_ipc::AbandonRequest {
            transaction_id: transaction_id.clone(),
            service: "DiagTrack".to_owned(),
        }),
        Command::RecoveryStatus,
    ] {
        dispatch(&commands, &frame(1, command));
    }

    assert_eq!(
        *calls.read(),
        vec![
            "preview",
            "apply",
            "undo",
            "restore_transaction",
            "restore_all",
            "abandon",
            "recovery_status"
        ]
    );
}

#[test]
fn an_oversized_response_is_rejected_instead_of_sending_an_empty_frame() {
    let commands = HugeReport::with_entries(600);

    let response = dispatch(&commands, &frame(21, Command::RestoreAll));
    let response =
        decode_response(&response).expect("an oversized response is replaced by a rejection frame");

    assert_eq!(response.request_id, 21);
    assert_eq!(
        response.body,
        ResponseBody::Rejected(optimi_ipc::ProtocolError::FrameTooLarge {
            limit: MAX_FRAME_BYTES
        })
    );
}

#[test]
fn a_restore_confirmation_travels_with_the_typed_command() {
    let (commands, calls) = RecordingCommands::new();
    let transaction_id = TransactionId::parse("tx-1").expect("transaction id is valid");

    dispatch(
        &commands,
        &frame(
            1,
            Command::RestoreTransaction(optimi_ipc::RestoreTransactionRequest {
                transaction_id,
                confirmed_build: Some(22631),
            }),
        ),
    );

    assert_eq!(*calls.read(), vec!["restore_transaction"]);
}
