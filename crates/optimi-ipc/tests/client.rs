mod support;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use optimi_domain::command::CommandOutcome;
use optimi_domain::id::{PreviewId, TransactionId};
use optimi_ipc::client::{TransportError, WorkerClient, WorkerError, WorkerTransport};
use optimi_ipc::{
    AbandonRequest, ApplyRequest, Command, PROTOCOL_VERSION, PreviewRequest, ResponseBody,
    RestoreTransactionRequest, UndoRequest,
};

#[derive(Default)]
struct TransportLog {
    starts: usize,
    closes: usize,
    frames: Vec<Vec<u8>>,
}

#[derive(Clone)]
struct Log(Arc<Mutex<TransportLog>>);

impl Log {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(TransportLog::default())))
    }

    fn read(&self) -> MutexGuard<'_, TransportLog> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn commands(&self) -> Vec<Command> {
        self.read()
            .frames
            .iter()
            .map(|frame| {
                optimi_ipc::decode_request(frame)
                    .expect("sent frame is valid")
                    .command
            })
            .collect()
    }
}

struct FakeTransport {
    log: Log,
    start_result: Result<(), TransportError>,
    replies: Vec<Result<Vec<u8>, TransportError>>,
}

impl FakeTransport {
    fn new(start_result: Result<(), TransportError>) -> Self {
        Self {
            log: Log::new(),
            start_result,
            replies: Vec::new(),
        }
    }

    fn with_reply(mut self, reply: Vec<u8>) -> Self {
        self.replies.push(Ok(reply));
        self
    }

    fn with_error(mut self, error: TransportError) -> Self {
        self.replies.push(Err(error));
        self
    }
}

impl WorkerTransport for FakeTransport {
    fn start(&mut self) -> Result<(), TransportError> {
        self.log.read().starts += 1;
        self.start_result
    }

    fn exchange(&mut self, frame: &[u8]) -> Result<Vec<u8>, TransportError> {
        self.log.read().frames.push(frame.to_vec());
        if self.replies.is_empty() {
            return Err(TransportError::Disconnected);
        }
        self.replies.remove(0)
    }

    fn close(&mut self) {
        self.log.read().closes += 1;
    }
}

fn client(start_result: Result<(), TransportError>) -> (WorkerClient<FakeTransport>, Log) {
    let transport = FakeTransport::new(start_result);
    let log = transport.log.clone();
    (WorkerClient::new(transport), log)
}

#[test]
fn apply_reviewed_previews_over_the_worker_and_applies_the_worker_preview() {
    let reviewed = support::preview("preview-1");
    let fresh_id = PreviewId::parse("preview-9").expect("preview id is valid");
    let transport = FakeTransport::new(Ok(()))
        .with_reply(support::response_frame(
            ResponseBody::Preview(CommandOutcome::Completed(support::preview("preview-9"))),
            1,
        ))
        .with_reply(support::response_frame(
            ResponseBody::Apply(CommandOutcome::Completed(support::verified_change())),
            2,
        ));
    let log = transport.log.clone();
    let mut client = WorkerClient::new(transport);

    let outcome = client
        .apply_reviewed(&reviewed)
        .expect("the worker applies the reviewed preview");

    assert_eq!(
        outcome,
        CommandOutcome::Completed(support::verified_change())
    );
    let commands = log.commands();
    assert_eq!(commands.len(), 2);
    assert_eq!(
        commands[0],
        Command::Preview(PreviewRequest {
            profile_id: support::profile_id()
        })
    );
    assert_eq!(
        commands[1],
        Command::Apply(ApplyRequest {
            preview_id: fresh_id.clone(),
            consent: support::consent(fresh_id)
        })
    );
    assert_eq!(log.read().starts, 1);
}

#[test]
fn apply_does_not_send_an_apply_when_the_worker_preview_changed() {
    let reviewed = support::preview("preview-1");
    let mut changed = support::preview("preview-9");
    changed.actions[0].observed = optimi_domain::model::Observation::observed(support::target());
    let transport = FakeTransport::new(Ok(())).with_reply(support::response_frame(
        ResponseBody::Preview(CommandOutcome::Completed(changed)),
        1,
    ));
    let log = transport.log.clone();
    let mut client = WorkerClient::new(transport);

    let outcome = client
        .apply_reviewed(&reviewed)
        .expect("a stale preview is a refusal, not a transport failure");

    assert_eq!(
        outcome,
        CommandOutcome::Refused(optimi_domain::command::Refusal::PreviewStateChanged {
            preview_id: reviewed.id.to_string()
        })
    );
    assert_eq!(log.commands().len(), 1);
}

#[test]
fn cancelling_elevation_aborts_without_exchanging_a_single_frame() {
    let reviewed = support::preview("preview-1");
    let (mut client, log) = client(Err(TransportError::ElevationCancelled));

    let error = client
        .apply_reviewed(&reviewed)
        .expect_err("cancelling elevation cannot succeed");

    assert_eq!(error, WorkerError::ElevationCancelled);
    let log = log.read();
    assert_eq!(log.frames.len(), 0);
    assert_eq!(log.starts, 1);
}

#[test]
fn a_worker_that_dies_mid_request_is_reported_and_the_session_closes() {
    let transport = FakeTransport::new(Ok(())).with_error(TransportError::Disconnected);
    let log = transport.log.clone();
    let mut client = WorkerClient::new(transport);

    let error = client
        .recovery_status()
        .expect_err("a dead worker cannot produce a reply");

    assert_eq!(error, WorkerError::Disconnected);
    assert_eq!(log.read().closes, 1);
}

#[test]
fn a_dead_worker_session_restarts_on_the_next_command() {
    let transport = FakeTransport::new(Ok(()))
        .with_error(TransportError::Disconnected)
        .with_reply(support::response_frame(
            ResponseBody::RecoveryStatus(CommandOutcome::Completed(support::recovery_status())),
            2,
        ));
    let log = transport.log.clone();
    let mut client = WorkerClient::new(transport);

    client
        .recovery_status()
        .expect_err("the first command loses the worker");
    let outcome = client
        .recovery_status()
        .expect("the next command starts a new worker session");

    assert_eq!(
        outcome,
        CommandOutcome::Completed(support::recovery_status())
    );
    assert_eq!(log.read().starts, 2);
}

#[test]
fn a_reply_for_another_request_is_rejected() {
    let transport = FakeTransport::new(Ok(())).with_reply(support::response_frame(
        ResponseBody::RecoveryStatus(CommandOutcome::Completed(support::recovery_status())),
        99,
    ));
    let mut client = WorkerClient::new(transport);

    let error = client
        .recovery_status()
        .expect_err("a mismatched request id is not a reply");

    assert_eq!(error, WorkerError::UnexpectedReply);
}

#[test]
fn a_worker_protocol_error_is_rejected_before_the_domain() {
    let transport = FakeTransport::new(Ok(())).with_reply(support::response_frame(
        ResponseBody::Rejected(optimi_ipc::ProtocolError::Malformed),
        1,
    ));
    let mut client = WorkerClient::new(transport);

    let error = client
        .recovery_status()
        .expect_err("a protocol rejection is not a successful command");

    assert_eq!(
        error,
        WorkerError::Rejected(optimi_ipc::ProtocolError::Malformed)
    );
}

#[test]
fn a_response_with_another_protocol_version_is_rejected() {
    let response = optimi_ipc::Response {
        protocol_version: PROTOCOL_VERSION + 1,
        request_id: 1,
        body: ResponseBody::RecoveryStatus(CommandOutcome::Completed(support::recovery_status())),
    };
    let transport = FakeTransport::new(Ok(()))
        .with_reply(optimi_ipc::encode_response(&response).expect("response encodes"));
    let mut client = WorkerClient::new(transport);

    let error = client
        .recovery_status()
        .expect_err("a mismatched protocol version cannot succeed");

    assert!(matches!(error, WorkerError::Protocol(_)));
}

#[test]
fn undo_restore_and_abandon_send_typed_domain_commands() {
    let replies = [
        support::response_frame(
            ResponseBody::Undo(CommandOutcome::Completed(support::verified_change())),
            1,
        ),
        support::response_frame(
            ResponseBody::Restore(CommandOutcome::Completed(support::verified_change())),
            2,
        ),
        support::response_frame(
            ResponseBody::Abandon(CommandOutcome::Completed(support::abandoned())),
            3,
        ),
    ];
    let transport = FakeTransport::new(Ok(()))
        .with_reply(replies[0].clone())
        .with_reply(replies[1].clone())
        .with_reply(replies[2].clone());
    let log = transport.log.clone();
    let mut client = WorkerClient::new(transport);
    let transaction_id = TransactionId::parse("tx-1").expect("transaction id is valid");

    client.undo(&transaction_id).expect("undo replies");
    client
        .restore(&transaction_id, Some(22631))
        .expect("restore replies");
    client
        .abandon(&transaction_id, "DiagTrack")
        .expect("abandon replies");

    let id = || TransactionId::parse("tx-1").expect("transaction id is valid");
    assert_eq!(
        log.commands(),
        vec![
            Command::Undo(UndoRequest {
                transaction_id: id()
            }),
            Command::RestoreTransaction(RestoreTransactionRequest {
                transaction_id: id(),
                confirmed_build: Some(22631)
            }),
            Command::Abandon(AbandonRequest {
                transaction_id: id(),
                service: "DiagTrack".to_owned()
            }),
        ]
    );
}

#[test]
fn a_worker_preview_refusal_is_not_an_apply() {
    let reviewed = support::preview("preview-1");
    let transport = FakeTransport::new(Ok(())).with_reply(support::response_frame(
        ResponseBody::Preview(CommandOutcome::Refused(
            optimi_domain::command::Refusal::UnknownProfile {
                profile_id: "competitive".to_owned(),
            },
        )),
        1,
    ));
    let log = transport.log.clone();
    let mut client = WorkerClient::new(transport);

    let error = client
        .apply_reviewed(&reviewed)
        .expect_err("a worker preview refusal is not an apply");

    assert!(matches!(error, WorkerError::PreviewRefused(_)));
    assert_eq!(log.commands().len(), 1);
}

#[test]
fn request_ids_increase_across_calls() {
    let transport = FakeTransport::new(Ok(()))
        .with_reply(support::response_frame(
            ResponseBody::Preview(CommandOutcome::Completed(support::preview("preview-1"))),
            1,
        ))
        .with_reply(support::response_frame(
            ResponseBody::Preview(CommandOutcome::Completed(support::preview("preview-2"))),
            2,
        ));
    let log = transport.log.clone();
    let mut client = WorkerClient::new(transport);

    client
        .preview(&support::profile_id())
        .expect("first preview replies");
    client
        .preview(&support::profile_id())
        .expect("second preview replies");

    let ids: Vec<u64> = log
        .read()
        .frames
        .iter()
        .map(|frame| {
            optimi_ipc::decode_request(frame)
                .expect("frame is valid")
                .request_id
        })
        .collect();
    assert_eq!(ids, vec![1, 2]);
}

#[test]
fn a_preview_id_cannot_be_swapped_into_consent_on_the_wire() {
    let preview_id = PreviewId::parse("preview-1").expect("preview id is valid");
    let consent = support::consent(PreviewId::parse("preview-2").expect("preview id is valid"));
    let transport = FakeTransport::new(Ok(())).with_reply(support::response_frame(
        ResponseBody::Apply(CommandOutcome::Refused(
            optimi_domain::command::Refusal::ConsentMismatch,
        )),
        1,
    ));
    let mut client = WorkerClient::new(transport);

    let outcome = client
        .apply(&preview_id, &consent)
        .expect("the worker refuses the mismatched consent");

    assert_eq!(
        outcome,
        CommandOutcome::Refused(optimi_domain::command::Refusal::ConsentMismatch)
    );
}
