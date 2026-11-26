use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use optimi_domain::adapter::SystemAdapter;
use optimi_domain::catalog::Catalog;
use optimi_domain::command::{CommandOutcome, Consent, DomainService, Refusal};
use optimi_domain::id::{ActionId, PreviewId, ProfileId, TransactionId};
use optimi_domain::journal::{JournalError, JournalRecord, JournalStore, ReadOnlyJournal};
use optimi_domain::model::{
    ActivePowerPlan, AntiCheatState, Observation, Preview, ReadFailure, ServiceRunState,
    ServiceStartType, ServiceState, VerifiedChange, WindowsIdentity, WindowsProductType,
    WindowsVersion, WriteFailure,
};
use optimi_ez::session::{Session, UiOutcome};
use optimi_ipc::client::{TransportError, WorkerClient, WorkerTransport};
use optimi_ipc::{Command, PROTOCOL_VERSION, Response, ResponseBody};

#[derive(Clone, Default)]
struct SharedJournal(Arc<Mutex<Vec<JournalRecord>>>);

impl SharedJournal {
    fn read(&self) -> MutexGuard<'_, Vec<JournalRecord>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl JournalStore for SharedJournal {
    fn append(&self, record: JournalRecord) -> Result<(), JournalError> {
        self.read().push(record);
        Ok(())
    }

    fn update(&self, record: JournalRecord) -> Result<(), JournalError> {
        let mut records = self.read();
        let Some(existing) = records
            .iter_mut()
            .find(|existing| existing.transaction_id == record.transaction_id)
        else {
            return Err(JournalError::MissingRecord);
        };
        *existing = record;
        Ok(())
    }

    fn records(&self) -> Result<Vec<JournalRecord>, JournalError> {
        Ok(self.read().clone())
    }
}

struct FakeAdapter {
    writes: Arc<AtomicUsize>,
}

impl FakeAdapter {
    fn state(run_state: ServiceRunState, start_type: ServiceStartType) -> ServiceState {
        ServiceState {
            run_state,
            start_type,
            delayed_start: run_state == ServiceRunState::Running,
        }
    }

    fn running() -> ServiceState {
        Self::state(ServiceRunState::Running, ServiceStartType::Automatic)
    }
}

impl SystemAdapter for FakeAdapter {
    fn windows_version(&self) -> Result<WindowsVersion, ReadFailure> {
        Ok(WindowsVersion {
            major: 10,
            minor: 0,
            build: 22631,
            product_type: WindowsProductType::Workstation,
        })
    }

    fn interactive_identity(&self) -> Result<WindowsIdentity, ReadFailure> {
        Ok(WindowsIdentity {
            account: "player".to_owned(),
            domain: Some("TEST".to_owned()),
        })
    }

    fn service_state(&self, service: &str) -> Result<ServiceState, ReadFailure> {
        match service {
            "DiagTrack" => Ok(Self::running()),
            _ => Err(ReadFailure::NotFound),
        }
    }

    fn active_power_plan(&self) -> Result<ActivePowerPlan, ReadFailure> {
        Ok(ActivePowerPlan {
            guid: "381b4222-f694-41f0-9685-ff5bb260df2e".to_owned(),
            name: Some("Balanced".to_owned()),
        })
    }

    fn anti_cheat_state(&self) -> Result<AntiCheatState, ReadFailure> {
        Ok(AntiCheatState {
            active: false,
            installed: false,
        })
    }

    fn write_service_state(&self, _: &str, _: ServiceState) -> Result<(), WriteFailure> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        Err(WriteFailure::AccessDenied)
    }
}

#[derive(Default)]
struct TransportLog {
    starts: usize,
    closes: usize,
    frames: Vec<Vec<u8>>,
}

#[derive(Clone)]
struct Log(Arc<Mutex<TransportLog>>);

impl Log {
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

type Replies = Arc<Mutex<VecDeque<Result<Vec<u8>, TransportError>>>>;

#[derive(Clone)]
struct Script {
    log: Log,
    replies: Replies,
}

impl Script {
    fn push(&self, reply: Vec<u8>) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Ok(reply));
    }

    fn push_error(&self, error: TransportError) {
        self.replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(Err(error));
    }
}

struct ScriptedTransport {
    script: Script,
    start_result: Result<(), TransportError>,
}

impl WorkerTransport for ScriptedTransport {
    fn start(&mut self) -> Result<(), TransportError> {
        self.script.log.read().starts += 1;
        self.start_result
    }

    fn exchange(&mut self, frame: &[u8]) -> Result<Vec<u8>, TransportError> {
        self.script.log.read().frames.push(frame.to_vec());
        let next = self
            .script
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        next.unwrap_or(Err(TransportError::Disconnected))
    }

    fn close(&mut self) {
        self.script.log.read().closes += 1;
    }
}

struct Harness {
    session: Session<FakeAdapter, SharedJournal, WorkerClient<ScriptedTransport>>,
    script: Script,
    journal: SharedJournal,
    writes: Arc<AtomicUsize>,
}

fn harness(start_result: Result<(), TransportError>) -> Harness {
    let log = Log(Arc::new(Mutex::new(TransportLog::default())));
    let script = Script {
        log,
        replies: Arc::new(Mutex::new(VecDeque::new())),
    };
    let journal = SharedJournal::default();
    let writes = Arc::new(AtomicUsize::new(0));
    let transport = ScriptedTransport {
        script: script.clone(),
        start_result,
    };
    let session = Session::new(
        FakeAdapter {
            writes: Arc::clone(&writes),
        },
        journal.clone(),
        WorkerClient::new(transport),
    );
    Harness {
        session,
        script,
        journal,
        writes,
    }
}

fn profile() -> ProfileId {
    ProfileId::parse("competitive").expect("profile id is valid")
}

fn preview_reply(preview: Preview, request_id: u64) -> Vec<u8> {
    optimi_ipc::encode_response(&Response {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        body: ResponseBody::Preview(CommandOutcome::Completed(preview)),
    })
    .expect("response encodes")
}

fn apply_reply(change: VerifiedChange, request_id: u64) -> Vec<u8> {
    optimi_ipc::encode_response(&Response {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        body: ResponseBody::Apply(CommandOutcome::Completed(change)),
    })
    .expect("response encodes")
}

#[test]
fn preview_reads_the_system_without_starting_the_worker() {
    let mut harness = harness(Ok(()));

    let outcome = harness.session.preview(&profile());

    assert!(matches!(outcome, CommandOutcome::Completed(_)));
    assert_eq!(harness.script.log.read().starts, 0);
    assert_eq!(harness.script.log.read().frames.len(), 0);
    assert_eq!(harness.writes.load(Ordering::SeqCst), 0);
    assert!(harness.journal.read().is_empty());
}

#[test]
fn recovery_status_reads_the_journal_without_starting_the_worker() {
    let harness = harness(Ok(()));

    let outcome = harness.session.recovery_status();

    assert_eq!(
        outcome,
        CommandOutcome::Completed(optimi_domain::command::RecoveryStatus {
            pending_transactions: Vec::new(),
            writes_permitted: true,
            blocker: None,
        })
    );
    assert_eq!(harness.script.log.read().starts, 0);
}

#[test]
fn apply_hands_the_reviewed_preview_to_the_worker_and_never_writes_locally() {
    let mut harness = harness(Ok(()));
    let reviewed = match harness.session.preview(&profile()) {
        CommandOutcome::Completed(preview) => preview,
        other => panic!("preview must complete, found {other:?}"),
    };
    let mut worker_preview = reviewed.clone();
    worker_preview.id = PreviewId::parse("preview-2").expect("preview id is valid");
    let change = VerifiedChange {
        transaction_id: TransactionId::parse("tx-1").expect("transaction id is valid"),
        action_id: ActionId::parse("diagtrack-privacy").expect("action id is valid"),
        service: "DiagTrack".to_owned(),
        verified_state: worker_preview.actions[0].target,
    };
    harness
        .script
        .push(preview_reply(worker_preview.clone(), 1));
    harness.script.push(apply_reply(change.clone(), 2));

    let outcome = harness.session.apply(
        &reviewed.id,
        &Consent {
            preview_id: reviewed.id.clone(),
        },
    );

    assert_eq!(outcome, UiOutcome::Completed(change));
    assert_eq!(harness.writes.load(Ordering::SeqCst), 0);
    assert!(harness.journal.read().is_empty());
    let commands = harness.script.log.commands();
    assert_eq!(commands.len(), 2);
    assert_eq!(
        commands[1],
        Command::Apply(optimi_ipc::ApplyRequest {
            preview_id: worker_preview.id.clone(),
            consent: Consent {
                preview_id: worker_preview.id.clone()
            }
        })
    );
}

#[test]
fn cancelling_elevation_aborts_without_writing_or_pending_a_transaction() {
    let mut harness = harness(Err(TransportError::ElevationCancelled));
    let reviewed = match harness.session.preview(&profile()) {
        CommandOutcome::Completed(preview) => preview,
        other => panic!("preview must complete, found {other:?}"),
    };

    let outcome = harness.session.apply(
        &reviewed.id,
        &Consent {
            preview_id: reviewed.id.clone(),
        },
    );

    assert_eq!(outcome, UiOutcome::ElevationCancelled);
    assert_eq!(harness.writes.load(Ordering::SeqCst), 0);
    assert!(harness.journal.read().is_empty());
    assert!(harness.script.log.read().frames.is_empty());
}

#[test]
fn a_worker_that_dies_mid_command_is_reported_and_never_counted_as_success() {
    let mut harness = harness(Ok(()));
    let reviewed = match harness.session.preview(&profile()) {
        CommandOutcome::Completed(preview) => preview,
        other => panic!("preview must complete, found {other:?}"),
    };
    harness.script.push_error(TransportError::Disconnected);

    let outcome = harness.session.apply(
        &reviewed.id,
        &Consent {
            preview_id: reviewed.id.clone(),
        },
    );

    assert_eq!(
        outcome,
        UiOutcome::WorkerUnavailable {
            detail: "disconnected"
        }
    );
    assert_eq!(harness.writes.load(Ordering::SeqCst), 0);
    assert!(harness.journal.read().is_empty());
}

#[test]
fn an_unavailable_worker_is_reported_without_touching_the_journal() {
    let mut harness = harness(Err(TransportError::Unavailable));
    let reviewed = match harness.session.preview(&profile()) {
        CommandOutcome::Completed(preview) => preview,
        other => panic!("preview must complete, found {other:?}"),
    };

    let outcome = harness.session.apply(
        &reviewed.id,
        &Consent {
            preview_id: reviewed.id.clone(),
        },
    );

    assert_eq!(
        outcome,
        UiOutcome::WorkerUnavailable {
            detail: "unavailable"
        }
    );
    assert!(harness.journal.read().is_empty());
}

#[test]
fn a_consent_mismatch_is_refused_before_the_worker_is_contacted() {
    let mut harness = harness(Ok(()));
    let reviewed = match harness.session.preview(&profile()) {
        CommandOutcome::Completed(preview) => preview,
        other => panic!("preview must complete, found {other:?}"),
    };

    let outcome = harness.session.apply(
        &reviewed.id,
        &Consent {
            preview_id: PreviewId::parse("preview-2").expect("preview id is valid"),
        },
    );

    assert_eq!(outcome, UiOutcome::Refused(Refusal::ConsentMismatch));
    assert_eq!(harness.script.log.read().starts, 0);
    assert!(harness.journal.read().is_empty());
}

#[test]
fn an_unknown_preview_is_refused_before_the_worker_is_contacted() {
    let mut harness = harness(Ok(()));

    let outcome = harness.session.apply(
        &PreviewId::parse("preview-9").expect("preview id is valid"),
        &Consent {
            preview_id: PreviewId::parse("preview-9").expect("preview id is valid"),
        },
    );

    assert_eq!(
        outcome,
        UiOutcome::Refused(Refusal::PreviewNotCurrent {
            preview_id: "preview-9".to_owned()
        })
    );
    assert_eq!(harness.script.log.read().starts, 0);
}

#[test]
fn the_session_builds_the_same_catalog_preview_the_domain_exposes() {
    let adapter = FakeAdapter {
        writes: Arc::new(AtomicUsize::new(0)),
    };
    let domain = DomainService::new(adapter, SharedJournal::default());
    let mut harness = harness(Ok(()));

    let expected = match domain.preview(&profile()) {
        CommandOutcome::Completed(preview) => preview,
        other => panic!("preview must complete, found {other:?}"),
    };
    let actual = match harness.session.preview(&profile()) {
        CommandOutcome::Completed(preview) => preview,
        other => panic!("preview must complete, found {other:?}"),
    };

    assert_eq!(actual.profile_id, expected.profile_id);
    assert_eq!(actual.catalog_version, expected.catalog_version);
    assert_eq!(actual.platform, expected.platform);
    assert_eq!(actual.actions, expected.actions);
    assert_eq!(
        actual.actions[0].observed,
        Observation::observed(FakeAdapter::running())
    );
    assert_eq!(Catalog::embedded().version, expected.catalog_version);
}

#[test]
fn closing_the_session_closes_the_worker_connection() {
    let mut harness = harness(Ok(()));
    let reviewed = match harness.session.preview(&profile()) {
        CommandOutcome::Completed(preview) => preview,
        other => panic!("preview must complete, found {other:?}"),
    };
    let mut worker_preview = reviewed.clone();
    worker_preview.id = PreviewId::parse("preview-2").expect("preview id is valid");
    let change = VerifiedChange {
        transaction_id: TransactionId::parse("tx-1").expect("transaction id is valid"),
        action_id: ActionId::parse("diagtrack-privacy").expect("action id is valid"),
        service: "DiagTrack".to_owned(),
        verified_state: worker_preview.actions[0].target,
    };
    harness.script.push(preview_reply(worker_preview, 1));
    harness.script.push(apply_reply(change, 2));
    let consent = Consent {
        preview_id: reviewed.id.clone(),
    };
    harness.session.apply(&reviewed.id, &consent);
    assert_eq!(harness.script.log.read().closes, 0);

    harness.session.shutdown();

    assert_eq!(harness.script.log.read().closes, 1);
}

struct ReadOnlyHarness {
    session: Session<FakeAdapter, ReadOnlyJournal, WorkerClient<ScriptedTransport>>,
    path: std::path::PathBuf,
}

fn read_only_harness() -> ReadOnlyHarness {
    let path = std::env::temp_dir().join(format!(
        "optimi-ui-journal-{}-{:?}.json",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(
        &path, b"[]
",
    )
    .expect("seeded journal is written");
    let log = Log(Arc::new(Mutex::new(TransportLog::default())));
    let script = Script {
        log,
        replies: Arc::new(Mutex::new(VecDeque::new())),
    };
    let transport = ScriptedTransport {
        script,
        start_result: Err(TransportError::Unavailable),
    };
    let session = Session::new(
        FakeAdapter {
            writes: Arc::new(AtomicUsize::new(0)),
        },
        ReadOnlyJournal::open(&path),
        WorkerClient::new(transport),
    );
    ReadOnlyHarness { session, path }
}

#[test]
fn the_unprivileged_session_cannot_write_the_machine_journal() {
    let mut harness = read_only_harness();
    let seeded = std::fs::read(&harness.path).expect("seeded journal is readable");

    let status = harness.session.recovery_status();
    let reviewed = match harness.session.preview(&profile()) {
        CommandOutcome::Completed(preview) => preview,
        other => panic!("preview must complete, found {other:?}"),
    };
    let outcome = harness.session.apply(
        &reviewed.id,
        &Consent {
            preview_id: reviewed.id.clone(),
        },
    );

    assert!(matches!(outcome, UiOutcome::WorkerUnavailable { .. }));
    assert_eq!(
        status,
        CommandOutcome::Completed(optimi_domain::command::RecoveryStatus {
            pending_transactions: Vec::new(),
            writes_permitted: true,
            blocker: None,
        })
    );
    assert_eq!(
        std::fs::read(&harness.path).expect("journal is still readable"),
        seeded
    );
    std::fs::remove_file(&harness.path).expect("seed file is removed");
}
