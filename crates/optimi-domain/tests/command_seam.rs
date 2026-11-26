use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex, MutexGuard, PoisonError};
use std::thread;

use optimi_domain::adapter::SystemAdapter;
use optimi_domain::catalog::{Catalog, CatalogAction, Profile};
use optimi_domain::command::{
    CommandFailure, CommandOutcome, Consent, DomainService, RecoveryStatus, Refusal, WriteBlocker,
};
use optimi_domain::id::{ActionId, PreviewId, ProfileId, TransactionId};
use optimi_domain::journal::{
    FileJournal, JournalError, JournalRecord, JournalStore, MemoryJournal, TransactionState,
};
use optimi_domain::model::{
    ActivePowerPlan, AntiCheatState, Disposition, DispositionReason, Evidence, Gate,
    MeasurementStatus, Observation, PlatformSupport, Preview, ReadFailure, Risk, ServiceRunState,
    ServiceSnapshot, ServiceStartType, ServiceState, VerifiedChange, WindowsIdentity,
    WindowsProductType, WindowsVersion, WriteFailure,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum WriteMode {
    Apply,
    Ignore,
    StartTypeOnly,
    Fail(WriteFailure),
}

struct BlockingWrite {
    entered: Barrier,
    release: Barrier,
}

impl BlockingWrite {
    fn new() -> Self {
        Self {
            entered: Barrier::new(2),
            release: Barrier::new(2),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServiceWrite {
    service: String,
    target: ServiceState,
}

struct FakeInner {
    version: Result<WindowsVersion, ReadFailure>,
    identity: Result<WindowsIdentity, ReadFailure>,
    power_plan: Result<ActivePowerPlan, ReadFailure>,
    services: HashMap<String, Result<ServiceState, ReadFailure>>,
    anti_cheat: Result<AntiCheatState, ReadFailure>,
    write_mode: WriteMode,
    read_failure_after_write: Option<ReadFailure>,
    armed_read_failure: Option<ReadFailure>,
}

#[derive(Clone)]
struct FakeSystemAdapter {
    inner: Arc<Mutex<FakeInner>>,
    writes: Arc<Mutex<Vec<ServiceWrite>>>,
    events: Arc<Mutex<Vec<String>>>,
    blocking: Arc<Mutex<Option<Arc<BlockingWrite>>>>,
}

impl FakeSystemAdapter {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeInner {
                version: Ok(WindowsVersion {
                    major: 10,
                    minor: 0,
                    build: 22631,
                    product_type: WindowsProductType::Workstation,
                }),
                identity: Ok(WindowsIdentity {
                    account: "player".to_owned(),
                    domain: Some("TEST".to_owned()),
                }),
                power_plan: Ok(ActivePowerPlan {
                    guid: "381b4222-f694-41f0-9685-ff5bb260df2e".to_owned(),
                    name: Some("Balanced".to_owned()),
                }),
                services: HashMap::new(),
                anti_cheat: Ok(AntiCheatState {
                    active: false,
                    installed: false,
                }),
                write_mode: WriteMode::Apply,
                read_failure_after_write: None,
                armed_read_failure: None,
            })),
            writes: Arc::new(Mutex::new(Vec::new())),
            events: Arc::new(Mutex::new(Vec::new())),
            blocking: Arc::new(Mutex::new(None)),
        }
    }

    fn with_service(self, name: &str, state: ServiceState) -> Self {
        self.set_service(name, state);
        self
    }

    fn with_service_failure(self, name: &str, failure: ReadFailure) -> Self {
        lock(&self.inner)
            .services
            .insert(name.to_owned(), Err(failure));
        self
    }

    fn with_version(self, version: WindowsVersion) -> Self {
        lock(&self.inner).version = Ok(version);
        self
    }

    fn with_version_failure(self, failure: ReadFailure) -> Self {
        lock(&self.inner).version = Err(failure);
        self
    }

    fn with_identity_failure(self, failure: ReadFailure) -> Self {
        lock(&self.inner).identity = Err(failure);
        self
    }

    fn with_power_plan_failure(self, failure: ReadFailure) -> Self {
        lock(&self.inner).power_plan = Err(failure);
        self
    }

    fn with_anti_cheat(self, state: AntiCheatState) -> Self {
        lock(&self.inner).anti_cheat = Ok(state);
        self
    }

    fn set_service(&self, name: &str, state: ServiceState) {
        lock(&self.inner)
            .services
            .insert(name.to_owned(), Ok(state));
    }

    fn set_version(&self, version: WindowsVersion) {
        lock(&self.inner).version = Ok(version);
    }

    fn set_version_failure(&self, failure: ReadFailure) {
        lock(&self.inner).version = Err(failure);
    }

    fn set_identity(&self, identity: WindowsIdentity) {
        lock(&self.inner).identity = Ok(identity);
    }

    fn set_anti_cheat(&self, state: AntiCheatState) {
        lock(&self.inner).anti_cheat = Ok(state);
    }

    fn set_anti_cheat_failure(&self, failure: ReadFailure) {
        lock(&self.inner).anti_cheat = Err(failure);
    }

    fn set_write_mode(&self, mode: WriteMode) {
        lock(&self.inner).write_mode = mode;
    }

    fn fail_reads_after_write(&self, failure: ReadFailure) {
        lock(&self.inner).read_failure_after_write = Some(failure);
    }

    fn block_writes(&self) -> Arc<BlockingWrite> {
        let blocking = Arc::new(BlockingWrite::new());
        *lock(&self.blocking) = Some(blocking.clone());
        blocking
    }

    fn write_count(&self) -> usize {
        lock(&self.writes).len()
    }

    fn events(&self) -> Arc<Mutex<Vec<String>>> {
        self.events.clone()
    }
}

impl SystemAdapter for FakeSystemAdapter {
    fn windows_version(&self) -> Result<WindowsVersion, ReadFailure> {
        lock(&self.inner).version.clone()
    }

    fn interactive_identity(&self) -> Result<WindowsIdentity, ReadFailure> {
        lock(&self.inner).identity.clone()
    }

    fn service_state(&self, service: &str) -> Result<ServiceState, ReadFailure> {
        lock(&self.events).push(format!("read:{service}"));
        let mut inner = lock(&self.inner);
        if let Some(failure) = inner.armed_read_failure.take() {
            return Err(failure);
        }
        inner
            .services
            .get(service)
            .cloned()
            .unwrap_or(Err(ReadFailure::NotFound))
    }

    fn active_power_plan(&self) -> Result<ActivePowerPlan, ReadFailure> {
        lock(&self.inner).power_plan.clone()
    }

    fn anti_cheat_state(&self) -> Result<AntiCheatState, ReadFailure> {
        lock(&self.inner).anti_cheat
    }

    fn write_service_state(&self, service: &str, target: ServiceState) -> Result<(), WriteFailure> {
        lock(&self.events).push(format!("write:{service}"));
        if let Some(blocking) = lock(&self.blocking).clone() {
            blocking.entered.wait();
            blocking.release.wait();
        }
        lock(&self.writes).push(ServiceWrite {
            service: service.to_owned(),
            target,
        });
        let mut inner = lock(&self.inner);
        match inner.write_mode {
            WriteMode::Fail(failure) => Err(failure),
            WriteMode::Ignore => Ok(()),
            WriteMode::StartTypeOnly => {
                if let Some(Ok(state)) = inner.services.get_mut(service) {
                    state.start_type = target.start_type;
                }
                inner.armed_read_failure = inner.read_failure_after_write;
                Ok(())
            }
            WriteMode::Apply => {
                if let Some(Ok(state)) = inner.services.get_mut(service) {
                    *state = target;
                }
                inner.armed_read_failure = inner.read_failure_after_write;
                Ok(())
            }
        }
    }
}

#[derive(Clone)]
struct RecordingJournal {
    records: Arc<Mutex<Vec<JournalRecord>>>,
    events: Arc<Mutex<Vec<String>>>,
    fail_append: Arc<AtomicBool>,
    fail_update: Arc<AtomicBool>,
    fail_read: Arc<AtomicBool>,
}

impl RecordingJournal {
    fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
            events,
            fail_append: Arc::new(AtomicBool::new(false)),
            fail_update: Arc::new(AtomicBool::new(false)),
            fail_read: Arc::new(AtomicBool::new(false)),
        }
    }

    fn fail_appends(&self) {
        self.fail_append.store(true, Ordering::SeqCst);
    }

    fn fail_updates(&self) {
        self.fail_update.store(true, Ordering::SeqCst);
    }

    fn fail_reads(&self) {
        self.fail_read.store(true, Ordering::SeqCst);
    }

    fn snapshot(&self) -> Vec<JournalRecord> {
        lock(&self.records).clone()
    }
}

impl JournalStore for RecordingJournal {
    fn append(&self, record: JournalRecord) -> Result<(), JournalError> {
        if self.fail_append.load(Ordering::SeqCst) {
            return Err(JournalError::Io);
        }
        lock(&self.events).push("journal:append".to_owned());
        lock(&self.records).push(record);
        Ok(())
    }

    fn update(&self, record: JournalRecord) -> Result<(), JournalError> {
        if self.fail_update.load(Ordering::SeqCst) {
            return Err(JournalError::Io);
        }
        let mut records = lock(&self.records);
        let Some(existing) = records
            .iter_mut()
            .find(|existing| existing.transaction_id == record.transaction_id)
        else {
            return Err(JournalError::MissingRecord);
        };
        lock(&self.events).push("journal:update".to_owned());
        *existing = record;
        Ok(())
    }

    fn records(&self) -> Result<Vec<JournalRecord>, JournalError> {
        if self.fail_read.load(Ordering::SeqCst) {
            return Err(JournalError::Io);
        }
        Ok(lock(&self.records).clone())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn service_state(run_state: ServiceRunState, start_type: ServiceStartType) -> ServiceState {
    ServiceState {
        run_state,
        start_type,
        delayed_start: false,
    }
}

fn delayed_service_state(
    run_state: ServiceRunState,
    start_type: ServiceStartType,
    delayed_start: bool,
) -> ServiceState {
    ServiceState {
        run_state,
        start_type,
        delayed_start,
    }
}

fn running_automatic() -> ServiceState {
    service_state(ServiceRunState::Running, ServiceStartType::Automatic)
}

fn target_state() -> ServiceState {
    service_state(ServiceRunState::Stopped, ServiceStartType::Disabled)
}

fn profile() -> ProfileId {
    ProfileId::parse("competitive").expect("profile id is valid")
}

fn ready_adapter() -> FakeSystemAdapter {
    FakeSystemAdapter::new().with_service("DiagTrack", running_automatic())
}

fn journaled_transaction(transaction_id: &str, state: TransactionState) -> JournalRecord {
    JournalRecord {
        transaction_id: TransactionId::parse(transaction_id).expect("transaction id is valid"),
        action_id: ActionId::parse("diagtrack-privacy").expect("action id is valid"),
        service: "DiagTrack".to_owned(),
        snapshot: ServiceSnapshot {
            service: "DiagTrack".to_owned(),
            existed: true,
            state: Some(running_automatic()),
        },
        target: target_state(),
        windows_version: WindowsVersion {
            major: 10,
            minor: 0,
            build: 22631,
            product_type: WindowsProductType::Workstation,
        },
        interactive_identity: WindowsIdentity {
            account: "player".to_owned(),
            domain: Some("TEST".to_owned()),
        },
        state,
        observed: Observation::observed(running_automatic()),
    }
}

fn journaled_service_transaction(
    transaction_id: &str,
    service: &str,
    snapshot: ServiceState,
    target: ServiceState,
    state: TransactionState,
    observed: ServiceState,
) -> JournalRecord {
    JournalRecord {
        transaction_id: TransactionId::parse(transaction_id).expect("transaction id is valid"),
        action_id: ActionId::parse("diagtrack-privacy").expect("action id is valid"),
        service: service.to_owned(),
        snapshot: ServiceSnapshot {
            service: service.to_owned(),
            existed: true,
            state: Some(snapshot),
        },
        target,
        windows_version: WindowsVersion {
            major: 10,
            minor: 0,
            build: 22631,
            product_type: WindowsProductType::Workstation,
        },
        interactive_identity: WindowsIdentity {
            account: "player".to_owned(),
            domain: Some("TEST".to_owned()),
        },
        state,
        observed: Observation::observed(observed),
    }
}

fn completed_preview(outcome: CommandOutcome<Preview>) -> Preview {
    let CommandOutcome::Completed(preview) = outcome else {
        panic!("expected a completed preview, got {outcome:?}");
    };
    preview
}

fn completed_change(outcome: CommandOutcome<VerifiedChange>) -> VerifiedChange {
    let CommandOutcome::Completed(change) = outcome else {
        panic!("expected a completed change, got {outcome:?}");
    };
    change
}

fn completed_status(outcome: CommandOutcome<RecoveryStatus>) -> RecoveryStatus {
    let CommandOutcome::Completed(status) = outcome else {
        panic!("expected a completed recovery status, got {outcome:?}");
    };
    status
}

fn completed_changes(outcome: CommandOutcome<Vec<VerifiedChange>>) -> Vec<VerifiedChange> {
    let CommandOutcome::Completed(changes) = outcome else {
        panic!("expected completed changes, got {outcome:?}");
    };
    changes
}

fn pending_ids(status: &RecoveryStatus) -> Vec<&str> {
    status
        .pending_transactions
        .iter()
        .map(|transaction| transaction.transaction_id.as_str())
        .collect()
}

fn temporary_journal_path() -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "optimi-command-{}-{unique}.json",
        std::process::id()
    ))
}

fn remove_journal(path: &PathBuf) {
    let mut lock = path.as_os_str().to_owned();
    lock.push(".lock");
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(PathBuf::from(lock));
}

fn catalog_action(id: &str, service: &str, target: ServiceState) -> CatalogAction {
    CatalogAction {
        id: ActionId::parse(id).expect("action id is valid"),
        service: service.to_owned(),
        risk: Risk::Low,
        target,
        evidence: Evidence {
            source: "test fixture".to_owned(),
            url: "https://example.invalid/evidence".to_owned(),
            date: "2026-01-01".to_owned(),
            measurement: MeasurementStatus::NotMeasured,
            claim_key: "claim.diagtrack-privacy".to_owned(),
        },
    }
}

fn consent(preview: &Preview) -> Consent {
    Consent {
        preview_id: preview.id.clone(),
    }
}

fn recording_domain(
    adapter: FakeSystemAdapter,
) -> (
    DomainService<FakeSystemAdapter, RecordingJournal>,
    RecordingJournal,
) {
    let journal = RecordingJournal::new(adapter.events());
    let domain = DomainService::new(adapter, journal.clone());
    (domain, journal)
}

#[test]
fn preview_reports_observed_service_state() {
    let domain = DomainService::new(ready_adapter(), MemoryJournal::new());

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.platform, PlatformSupport::Supported);
    assert_eq!(preview.actions.len(), 1);
    assert_eq!(preview.actions[0].action_id.as_str(), "diagtrack-privacy");
    assert_eq!(preview.actions[0].disposition, Disposition::Applicable);
    assert_eq!(preview.actions[0].reason, DispositionReason::TargetDiffers);
    assert_eq!(preview.actions[0].target, target_state());
    assert!(matches!(
        &preview.actions[0].observed,
        Observation::Observed { value } if value.run_state == ServiceRunState::Running
            && value.start_type == ServiceStartType::Automatic
    ));
    assert!(matches!(
        &preview.windows_version,
        Observation::Observed { value } if value.build == 22631
    ));
    assert!(matches!(
        &preview.interactive_identity,
        Observation::Observed { value } if value.account == "player" && value.domain.as_deref() == Some("TEST")
    ));
    assert!(matches!(
        &preview.active_power_plan,
        Observation::Observed { value } if value.name.as_deref() == Some("Balanced")
    ));
}

#[test]
fn preview_marks_already_disabled_service_as_no_op() {
    let domain = DomainService::new(
        FakeSystemAdapter::new().with_service("DiagTrack", target_state()),
        MemoryJournal::new(),
    );

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.actions[0].disposition, Disposition::NoOp);
    assert_eq!(
        preview.actions[0].reason,
        DispositionReason::AlreadyAtTarget
    );
}

#[test]
fn preview_surfaces_service_read_failure_as_unknown() {
    let domain = DomainService::new(
        FakeSystemAdapter::new().with_service_failure("DiagTrack", ReadFailure::AccessDenied),
        MemoryJournal::new(),
    );

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.actions[0].disposition, Disposition::Unknown);
    assert_eq!(
        preview.actions[0].reason,
        DispositionReason::StateUnavailable
    );
    assert!(matches!(
        &preview.actions[0].observed,
        Observation::Unavailable {
            failure: ReadFailure::AccessDenied
        }
    ));
}

#[test]
fn preview_blocks_unsupported_windows_build() {
    let domain = DomainService::new(
        FakeSystemAdapter::new()
            .with_version(WindowsVersion {
                major: 10,
                minor: 0,
                build: 19045,
                product_type: WindowsProductType::Workstation,
            })
            .with_service("DiagTrack", running_automatic()),
        MemoryJournal::new(),
    );

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.platform, PlatformSupport::Unsupported);
    assert_eq!(preview.actions[0].disposition, Disposition::Blocked);
    assert_eq!(
        preview.actions[0].reason,
        DispositionReason::UnsupportedWindowsVersion
    );
}

#[test]
fn preview_blocks_windows_server_2025_on_its_own_build_number() {
    let domain = DomainService::new(
        FakeSystemAdapter::new()
            .with_version(WindowsVersion {
                major: 10,
                minor: 0,
                build: 26100,
                product_type: WindowsProductType::Server,
            })
            .with_service("DiagTrack", running_automatic()),
        MemoryJournal::new(),
    );

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.platform, PlatformSupport::Unsupported);
    assert_eq!(preview.actions[0].disposition, Disposition::Blocked);
    assert_eq!(
        preview.actions[0].reason,
        DispositionReason::UnsupportedWindowsVersion
    );
}

#[test]
fn preview_supports_windows_11_client_on_the_same_build_number() {
    let domain = DomainService::new(
        FakeSystemAdapter::new()
            .with_version(WindowsVersion {
                major: 10,
                minor: 0,
                build: 26100,
                product_type: WindowsProductType::Workstation,
            })
            .with_service("DiagTrack", running_automatic()),
        MemoryJournal::new(),
    );

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.platform, PlatformSupport::Supported);
    assert_eq!(preview.actions[0].disposition, Disposition::Applicable);
    assert_eq!(preview.actions[0].reason, DispositionReason::TargetDiffers);
}

#[test]
fn preview_reports_unknown_platform_when_version_read_fails() {
    let domain = DomainService::new(
        FakeSystemAdapter::new()
            .with_version_failure(ReadFailure::SystemError { code: 1 })
            .with_service("DiagTrack", running_automatic()),
        MemoryJournal::new(),
    );

    let preview = completed_preview(domain.preview(&profile()));

    assert_eq!(preview.platform, PlatformSupport::Unknown);
    assert_eq!(preview.actions[0].disposition, Disposition::Unknown);
    assert_eq!(
        preview.actions[0].reason,
        DispositionReason::PlatformUnknown
    );
}

#[test]
fn preview_keeps_other_reads_when_identity_and_power_plan_fail() {
    let domain = DomainService::new(
        FakeSystemAdapter::new()
            .with_identity_failure(ReadFailure::UnsupportedPlatform)
            .with_power_plan_failure(ReadFailure::NotFound)
            .with_service("DiagTrack", running_automatic()),
        MemoryJournal::new(),
    );

    let preview = completed_preview(domain.preview(&profile()));

    assert!(matches!(
        preview.interactive_identity,
        Observation::Unavailable {
            failure: ReadFailure::UnsupportedPlatform
        }
    ));
    assert!(matches!(
        preview.active_power_plan,
        Observation::Unavailable {
            failure: ReadFailure::NotFound
        }
    ));
    assert_eq!(preview.actions[0].disposition, Disposition::Applicable);
}

#[test]
fn apply_journals_the_exact_snapshot_before_writing() {
    let adapter = ready_adapter();
    let events = adapter.events();
    let (domain, journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));

    assert_eq!(change.transaction_id.as_str(), "tx-1");
    assert_eq!(change.service, "DiagTrack");
    assert_eq!(change.verified_state, target_state());
    assert_eq!(adapter.write_count(), 1);

    let log = lock(&events);
    let append = log
        .iter()
        .position(|event| event == "journal:append")
        .expect("journal append was recorded");
    let write = log
        .iter()
        .position(|event| event == "write:DiagTrack")
        .expect("service write was recorded");
    assert!(append < write, "journal append must precede the write");

    let records = journal.snapshot();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.snapshot.service, "DiagTrack");
    assert!(record.snapshot.existed);
    assert_eq!(record.snapshot.state, Some(running_automatic()));
    assert_eq!(record.target, target_state());
    assert_eq!(record.windows_version.build, 22631);
    assert_eq!(record.interactive_identity.account, "player");
    assert_eq!(record.state, TransactionState::Verified);
}

#[test]
fn apply_skips_transaction_ids_already_in_the_journal() {
    let adapter = ready_adapter();
    let journal = RecordingJournal::new(adapter.events());
    journal
        .append(journaled_transaction("tx-1", TransactionState::Verified))
        .expect("existing transaction is journaled");
    let domain = DomainService::new(adapter, journal.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));

    assert_eq!(change.transaction_id.as_str(), "tx-2");
    let records = journal.snapshot();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].state, TransactionState::Verified);
}

#[test]
fn apply_succeeds_only_after_reading_the_written_state_back() {
    let adapter = ready_adapter();
    let (domain, journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));

    assert_eq!(change.verified_state, target_state());
    assert_eq!(
        adapter
            .service_state("DiagTrack")
            .expect("service remains readable"),
        target_state()
    );
    let events = adapter.events();
    let log = lock(&events);
    let write = log
        .iter()
        .position(|event| event == "write:DiagTrack")
        .expect("write was recorded");
    assert!(
        log[write + 1..]
            .iter()
            .any(|event| event == "read:DiagTrack"),
        "apply must read the service after writing it"
    );
    assert_eq!(journal.snapshot()[0].state, TransactionState::Verified);
}

#[test]
fn apply_reports_recovery_required_when_read_back_does_not_match() {
    let adapter = ready_adapter();
    adapter.set_write_mode(WriteMode::Ignore);
    let (domain, journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&preview.id, &consent(&preview));

    let CommandOutcome::RecoveryRequired(status) = outcome else {
        panic!("expected recovery required, got {outcome:?}");
    };
    assert_eq!(status.pending_transactions.len(), 1);
    assert!(!status.writes_permitted);
    assert_eq!(status.blocker, Some(WriteBlocker::PendingRecovery));
    assert_eq!(adapter.write_count(), 1);
    assert_eq!(
        adapter
            .service_state("DiagTrack")
            .expect("service remains readable"),
        running_automatic()
    );
    assert_eq!(
        journal.snapshot()[0].state,
        TransactionState::RecoveryRequired
    );
}

#[test]
fn apply_reports_recovery_required_when_the_write_fails() {
    let adapter = ready_adapter();
    adapter.set_write_mode(WriteMode::Fail(WriteFailure::AccessDenied));
    let (domain, journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert!(matches!(outcome, CommandOutcome::RecoveryRequired(_)));
    assert_eq!(adapter.write_count(), 1);
    assert_eq!(
        adapter
            .service_state("DiagTrack")
            .expect("service remains readable"),
        running_automatic()
    );
    assert_eq!(
        journal.snapshot()[0].state,
        TransactionState::RecoveryRequired
    );
}

#[test]
fn apply_reports_recovery_required_when_the_read_back_fails() {
    let adapter = ready_adapter();
    adapter.fail_reads_after_write(ReadFailure::AccessDenied);
    let (domain, journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert!(matches!(outcome, CommandOutcome::RecoveryRequired(_)));
    let records = journal.snapshot();
    assert_eq!(records[0].state, TransactionState::RecoveryRequired);
    assert!(matches!(
        records[0].observed,
        Observation::Unavailable {
            failure: ReadFailure::AccessDenied
        }
    ));
}

#[test]
fn apply_fails_without_writing_when_the_journal_cannot_append() {
    let adapter = ready_adapter();
    let (domain, journal) = recording_domain(adapter.clone());
    journal.fail_appends();

    let preview = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Failed(CommandFailure::JournalWrite)
    );
    assert_eq!(adapter.write_count(), 0);
    assert!(journal.snapshot().is_empty());
}

#[test]
fn apply_refuses_a_preview_that_was_superseded() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let first = completed_preview(domain.preview(&profile()));
    let second = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&first.id, &consent(&first));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::PreviewNotCurrent {
            preview_id: first.id.to_string()
        })
    );
    assert_ne!(second.id, first.id);
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn apply_refuses_a_preview_that_was_already_consumed() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    completed_change(domain.apply(&preview.id, &consent(&preview)));
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::PreviewNotCurrent {
            preview_id: preview.id.to_string()
        })
    );
    assert_eq!(adapter.write_count(), 1);
}

#[test]
fn apply_refuses_an_unknown_preview() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());
    let preview_id = PreviewId::parse("preview-99").expect("preview id is valid");

    let outcome = domain.apply(
        &preview_id,
        &Consent {
            preview_id: preview_id.clone(),
        },
    );

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::PreviewNotCurrent {
            preview_id: "preview-99".to_owned()
        })
    );
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn apply_refuses_when_the_observed_service_state_changed_after_preview() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    adapter.set_service(
        "DiagTrack",
        service_state(ServiceRunState::Stopped, ServiceStartType::Manual),
    );
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::PreviewStateChanged {
            preview_id: preview.id.to_string()
        })
    );
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn apply_refuses_when_the_windows_build_changed_after_preview() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    adapter.set_version(WindowsVersion {
        major: 10,
        minor: 0,
        build: 26100,
        product_type: WindowsProductType::Workstation,
    });
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::PreviewStateChanged {
            preview_id: preview.id.to_string()
        })
    );
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn apply_refuses_when_the_interactive_identity_changed_after_preview() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    adapter.set_identity(WindowsIdentity {
        account: "someone-else".to_owned(),
        domain: Some("TEST".to_owned()),
    });
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::PreviewStateChanged {
            preview_id: preview.id.to_string()
        })
    );
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn apply_refuses_when_the_service_disappeared_after_preview() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    lock(&adapter.inner).services.remove("DiagTrack");
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::PreviewStateChanged {
            preview_id: preview.id.to_string()
        })
    );
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn apply_reports_recovery_required_when_verification_cannot_be_journaled() {
    let adapter = ready_adapter();
    let (domain, journal) = recording_domain(adapter.clone());
    journal.fail_updates();

    let preview = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert!(matches!(outcome, CommandOutcome::RecoveryRequired(_)));
    assert_eq!(adapter.write_count(), 1);
    assert_eq!(journal.snapshot()[0].state, TransactionState::Applying);
}

#[test]
fn apply_refuses_a_no_op_preview() {
    let adapter = FakeSystemAdapter::new().with_service("DiagTrack", target_state());
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::NoApplicableAction {
            preview_id: preview.id.to_string()
        })
    );
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn apply_refuses_when_the_platform_gate_fails_at_write_time() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    adapter.set_version(WindowsVersion {
        major: 10,
        minor: 0,
        build: 19045,
        product_type: WindowsProductType::Workstation,
    });
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::GateBlocked {
            gate: Gate::Platform
        })
    );
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn apply_fails_closed_when_the_platform_gate_cannot_read_the_version() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    adapter.set_version_failure(ReadFailure::SystemError { code: 5 });
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::GateReadFailed {
            gate: Gate::Platform
        })
    );
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn apply_refuses_when_anti_cheat_is_active() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    adapter.set_anti_cheat(AntiCheatState {
        active: true,
        installed: true,
    });
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::GateBlocked {
            gate: Gate::AntiCheat
        })
    );
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn apply_fails_closed_when_anti_cheat_state_cannot_be_read() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    adapter.set_anti_cheat_failure(ReadFailure::AccessDenied);
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::GateReadFailed {
            gate: Gate::AntiCheat
        })
    );
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn apply_allows_installed_but_stopped_anti_cheat() {
    let adapter = ready_adapter().with_anti_cheat(AntiCheatState {
        active: false,
        installed: true,
    });
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert!(matches!(outcome, CommandOutcome::Completed(_)));
    assert_eq!(adapter.write_count(), 1);
}

#[test]
fn apply_refuses_while_recovery_is_pending() {
    let adapter = ready_adapter();
    adapter.set_write_mode(WriteMode::Ignore);
    let (domain, _journal) = recording_domain(adapter.clone());

    let failed = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&failed.id, &consent(&failed));
    assert!(matches!(outcome, CommandOutcome::RecoveryRequired(_)));
    assert_eq!(adapter.write_count(), 1);

    adapter.set_write_mode(WriteMode::Apply);
    let next = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&next.id, &consent(&next));

    let CommandOutcome::RecoveryRequired(status) = outcome else {
        panic!("expected recovery required, got {outcome:?}");
    };
    assert_eq!(status.pending_transactions.len(), 1);
    assert!(!status.writes_permitted);
    assert_eq!(adapter.write_count(), 1);
}

#[test]
fn a_second_apply_is_refused_while_a_write_is_in_flight() {
    let adapter = ready_adapter();
    let blocking = adapter.block_writes();
    let domain = Arc::new(DomainService::new(adapter.clone(), MemoryJournal::new()));
    let preview = completed_preview(domain.preview(&profile()));
    let consent = consent(&preview);

    let first_domain = Arc::clone(&domain);
    let first_preview = preview.id.clone();
    let first_consent = consent.clone();
    let handle = thread::spawn(move || first_domain.apply(&first_preview, &first_consent));
    blocking.entered.wait();

    let second = domain.apply(&preview.id, &consent);
    assert_eq!(second, CommandOutcome::Refused(Refusal::WriteInProgress));

    blocking.release.wait();
    let first = handle.join().expect("first apply thread must finish");
    assert!(matches!(first, CommandOutcome::Completed(_)));
    assert_eq!(adapter.write_count(), 1);
}

#[test]
fn undo_restores_the_exact_snapshot_and_verifies_it() {
    let adapter = ready_adapter();
    let (domain, journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));
    let restored = completed_change(domain.undo(&change.transaction_id));

    assert_eq!(restored.transaction_id, change.transaction_id);
    assert_eq!(restored.verified_state, running_automatic());
    assert_eq!(
        adapter
            .service_state("DiagTrack")
            .expect("service remains readable"),
        running_automatic()
    );
    assert_eq!(adapter.write_count(), 2);
    let records = journal.snapshot();
    assert_eq!(records[0].state, TransactionState::Restored);
    assert_eq!(
        records[0].observed,
        Observation::observed(running_automatic())
    );
    let status = completed_status(domain.recovery_status());
    assert!(status.writes_permitted);
    assert!(status.pending_transactions.is_empty());
    assert_eq!(status.blocker, None);
}

#[test]
fn undo_reports_recovery_required_when_restore_cannot_be_verified() {
    let adapter = ready_adapter();
    let (domain, journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));
    adapter.set_write_mode(WriteMode::Ignore);
    let outcome = domain.undo(&change.transaction_id);

    let CommandOutcome::RecoveryRequired(status) = outcome else {
        panic!("expected recovery required, got {outcome:?}");
    };
    assert_eq!(pending_ids(&status), vec![change.transaction_id.as_str()]);
    assert_eq!(
        adapter
            .service_state("DiagTrack")
            .expect("service remains readable"),
        target_state()
    );
    assert_eq!(
        journal.snapshot()[0].state,
        TransactionState::RecoveryRequired
    );

    adapter.set_write_mode(WriteMode::Apply);
    let restored = completed_change(domain.undo(&change.transaction_id));
    assert_eq!(restored.verified_state, running_automatic());
    assert_eq!(journal.snapshot()[0].state, TransactionState::Restored);
    assert!(completed_status(domain.recovery_status()).writes_permitted);
}

#[test]
fn undo_refuses_an_unknown_transaction() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let outcome = domain.undo(&TransactionId::parse("tx-99").expect("transaction id is valid"));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::UnknownTransaction {
            transaction_id: "tx-99".to_owned()
        })
    );
    assert_eq!(adapter.write_count(), 0);
}

#[test]
fn undo_refuses_a_transaction_that_was_already_restored() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));
    completed_change(domain.undo(&change.transaction_id));
    let outcome = domain.undo(&change.transaction_id);

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::AlreadyRestored {
            transaction_id: change.transaction_id.to_string()
        })
    );
    assert_eq!(adapter.write_count(), 2);
}

#[test]
fn undo_rechecks_the_anti_cheat_gate() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));
    adapter.set_anti_cheat(AntiCheatState {
        active: true,
        installed: true,
    });
    let outcome = domain.undo(&change.transaction_id);

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::GateBlocked {
            gate: Gate::AntiCheat
        })
    );
    assert_eq!(adapter.write_count(), 1);
    assert_eq!(
        adapter
            .service_state("DiagTrack")
            .expect("service remains readable"),
        target_state()
    );
}

#[test]
fn recovery_status_is_clean_with_no_pending_transactions() {
    let domain = DomainService::new(ready_adapter(), MemoryJournal::new());

    let status = completed_status(domain.recovery_status());

    assert!(status.pending_transactions.is_empty());
    assert!(status.writes_permitted);
    assert_eq!(status.blocker, None);
}

#[test]
fn recovery_status_fails_closed_when_the_journal_cannot_be_read() {
    let adapter = ready_adapter();
    let (domain, journal) = recording_domain(adapter);
    journal.fail_reads();

    let status = completed_status(domain.recovery_status());

    assert!(status.pending_transactions.is_empty());
    assert!(!status.writes_permitted);
    assert_eq!(status.blocker, Some(WriteBlocker::JournalUnavailable));
}

#[test]
fn apply_refuses_consent_for_a_different_preview() {
    let domain = DomainService::new(ready_adapter(), MemoryJournal::new());

    let outcome = domain.apply(
        &PreviewId::parse("preview-1").expect("preview id is valid"),
        &Consent {
            preview_id: PreviewId::parse("preview-2").expect("preview id is valid"),
        },
    );

    assert_eq!(outcome, CommandOutcome::Refused(Refusal::ConsentMismatch));
}

#[test]
fn restore_all_restores_every_pending_transaction() {
    let adapter = FakeSystemAdapter::new()
        .with_service(
            "DiagTrack",
            service_state(ServiceRunState::Stopped, ServiceStartType::Disabled),
        )
        .with_service(
            "SysMain",
            service_state(ServiceRunState::Stopped, ServiceStartType::Disabled),
        );
    let journal = RecordingJournal::new(adapter.events());
    journal
        .append(journaled_service_transaction(
            "tx-1",
            "DiagTrack",
            running_automatic(),
            target_state(),
            TransactionState::RecoveryRequired,
            target_state(),
        ))
        .expect("first transaction is journaled");
    journal
        .append(journaled_service_transaction(
            "tx-2",
            "SysMain",
            service_state(ServiceRunState::Running, ServiceStartType::Manual),
            service_state(ServiceRunState::Stopped, ServiceStartType::Disabled),
            TransactionState::RecoveryRequired,
            service_state(ServiceRunState::Stopped, ServiceStartType::Disabled),
        ))
        .expect("second transaction is journaled");
    let domain = DomainService::new(adapter.clone(), journal.clone());

    let changes = completed_changes(domain.restore_all());

    assert_eq!(changes.len(), 2);
    assert_eq!(adapter.write_count(), 2);
    assert!(
        journal
            .snapshot()
            .iter()
            .all(|record| record.state == TransactionState::Restored)
    );
    assert!(completed_status(domain.recovery_status()).writes_permitted);
}

#[test]
fn unknown_profile_is_refused() {
    let domain = DomainService::new(FakeSystemAdapter::new(), MemoryJournal::new());

    let outcome = domain.preview(&ProfileId::parse("missing").expect("profile id is valid"));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::UnknownProfile {
            profile_id: "missing".to_owned()
        })
    );
}

#[test]
fn preview_ids_are_unique() {
    let domain = DomainService::new(ready_adapter(), MemoryJournal::new());

    let first = completed_preview(domain.preview(&profile()));
    let second = completed_preview(domain.preview(&profile()));

    assert_ne!(first.id, second.id);
}

#[test]
fn apply_recovers_through_undo_when_verification_failed() {
    let adapter = ready_adapter();
    adapter.set_write_mode(WriteMode::Ignore);
    let (domain, _journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&preview.id, &consent(&preview));
    let CommandOutcome::RecoveryRequired(status) = outcome else {
        panic!("expected recovery required, got {outcome:?}");
    };
    let transaction_id = status.pending_transactions[0].transaction_id.clone();

    adapter.set_write_mode(WriteMode::Apply);
    let restored = completed_change(domain.undo(&transaction_id));

    assert_eq!(restored.verified_state, running_automatic());
    assert!(completed_status(domain.recovery_status()).writes_permitted);
}

#[test]
fn apply_reports_recovery_required_for_a_partial_write() {
    let adapter = ready_adapter();
    adapter.set_write_mode(WriteMode::StartTypeOnly);
    let (domain, journal) = recording_domain(adapter.clone());

    let preview = completed_preview(domain.preview(&profile()));
    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert!(matches!(outcome, CommandOutcome::RecoveryRequired(_)));
    assert_eq!(
        journal.snapshot()[0].state,
        TransactionState::RecoveryRequired
    );
    assert_eq!(
        adapter
            .service_state("DiagTrack")
            .expect("service remains readable"),
        service_state(ServiceRunState::Running, ServiceStartType::Disabled)
    );
}

fn catalog_with(actions: Vec<CatalogAction>) -> Catalog {
    let action_ids = actions.iter().map(|action| action.id.clone()).collect();
    Catalog::new(
        "test",
        actions,
        vec![Profile {
            id: profile(),
            action_ids,
        }],
    )
}

#[test]
fn a_pending_transaction_is_restorable_after_the_service_restarts() {
    let path = temporary_journal_path();
    let adapter = ready_adapter();
    adapter.set_write_mode(WriteMode::Ignore);
    let domain = DomainService::new(adapter.clone(), FileJournal::new(&path));

    let preview = completed_preview(domain.preview(&profile()));
    assert!(matches!(
        domain.apply(&preview.id, &consent(&preview)),
        CommandOutcome::RecoveryRequired(_)
    ));
    let status = completed_status(domain.recovery_status());
    assert_eq!(status.pending_transactions.len(), 1);
    let pending = &status.pending_transactions[0];
    assert_eq!(pending.service, "DiagTrack");
    assert_eq!(pending.state, TransactionState::RecoveryRequired);
    assert_eq!(pending.snapshot.state, Some(running_automatic()));
    drop(domain);

    adapter.set_write_mode(WriteMode::Apply);
    let restarted = DomainService::new(adapter.clone(), FileJournal::new(&path));
    let status = completed_status(restarted.recovery_status());
    assert!(!status.writes_permitted);
    assert_eq!(pending_ids(&status), vec!["tx-1"]);
    let blocked_preview = completed_preview(restarted.preview(&profile()));
    assert!(matches!(
        restarted.apply(&blocked_preview.id, &consent(&blocked_preview)),
        CommandOutcome::RecoveryRequired(_)
    ));

    let restored = completed_change(
        restarted.restore_transaction(&status.pending_transactions[0].transaction_id),
    );

    assert_eq!(restored.verified_state, running_automatic());
    assert_eq!(adapter.write_count(), 1);
    let records = FileJournal::new(&path).records().expect("journal reads");
    assert_eq!(records[0].state, TransactionState::Restored);
    assert!(completed_status(restarted.recovery_status()).writes_permitted);
    remove_journal(&path);
}

#[test]
fn apply_refuses_a_transient_service_state_before_writing() {
    let adapter = FakeSystemAdapter::new().with_service(
        "DiagTrack",
        service_state(ServiceRunState::StartPending, ServiceStartType::Automatic),
    );
    let (domain, journal) = recording_domain(adapter.clone());
    let preview = completed_preview(domain.preview(&profile()));
    assert_eq!(preview.actions[0].disposition, Disposition::Applicable);

    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::ServiceStateNotSettled {
            service: "DiagTrack".to_owned(),
            state: ServiceRunState::StartPending,
        })
    );
    assert_eq!(adapter.write_count(), 0);
    assert!(journal.snapshot().is_empty());
}

#[cfg(windows)]
#[test]
fn a_second_domain_cannot_write_while_another_holds_the_journal_lock() {
    let path = temporary_journal_path();
    let adapter = ready_adapter();
    let first_domain = Arc::new(DomainService::new(adapter.clone(), FileJournal::new(&path)));
    let second_domain = DomainService::new(adapter.clone(), FileJournal::new(&path));
    let first_preview = completed_preview(first_domain.preview(&profile()));
    let second_preview = completed_preview(second_domain.preview(&profile()));
    let blocking = adapter.block_writes();
    let first_consent = consent(&first_preview);
    let handle = {
        let domain = Arc::clone(&first_domain);
        let preview_id = first_preview.id.clone();
        thread::spawn(move || domain.apply(&preview_id, &first_consent))
    };
    blocking.entered.wait();

    let second = second_domain.apply(&second_preview.id, &consent(&second_preview));
    assert_eq!(second, CommandOutcome::Refused(Refusal::WriteInProgress));

    blocking.release.wait();
    let first = completed_change(handle.join().expect("first apply thread must finish"));
    *lock(&adapter.blocking) = None;
    adapter.set_service("DiagTrack", running_automatic());
    let retry_preview = completed_preview(second_domain.preview(&profile()));
    let second = completed_change(second_domain.apply(&retry_preview.id, &consent(&retry_preview)));

    assert_ne!(first.transaction_id, second.transaction_id);
    let records = FileJournal::new(&path).records().expect("journal reads");
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|record| record.state == TransactionState::Verified)
    );
    remove_journal(&path);
}

#[test]
fn restore_recovers_a_partial_write_left_by_a_crash() {
    let partial = service_state(ServiceRunState::Running, ServiceStartType::Disabled);
    let adapter = FakeSystemAdapter::new().with_service("DiagTrack", partial);
    let journal = RecordingJournal::new(adapter.events());
    journal
        .append(journaled_service_transaction(
            "tx-1",
            "DiagTrack",
            running_automatic(),
            target_state(),
            TransactionState::Applying,
            running_automatic(),
        ))
        .expect("interrupted transaction is journaled");
    let domain = DomainService::new(adapter.clone(), journal.clone());
    let transaction_id = TransactionId::parse("tx-1").expect("transaction id is valid");

    let restored = completed_change(domain.restore_transaction(&transaction_id));

    assert_eq!(restored.verified_state, running_automatic());
    assert_eq!(
        adapter
            .service_state("DiagTrack")
            .expect("service remains readable"),
        running_automatic()
    );
    assert_eq!(adapter.write_count(), 1);
    assert_eq!(journal.snapshot()[0].state, TransactionState::Restored);
}

#[test]
fn undo_refuses_when_the_service_changed_outside_the_transaction() {
    let adapter = ready_adapter();
    let (domain, journal) = recording_domain(adapter.clone());
    let preview = completed_preview(domain.preview(&profile()));
    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));
    let external = service_state(ServiceRunState::Stopped, ServiceStartType::Manual);
    adapter.set_service("DiagTrack", external);

    let outcome = domain.undo(&change.transaction_id);

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::RestoreConflict {
            transaction_id: change.transaction_id.to_string(),
            expected: running_automatic(),
            current: external,
        })
    );
    assert_eq!(adapter.write_count(), 1);
    assert_eq!(
        adapter
            .service_state("DiagTrack")
            .expect("service remains readable"),
        external
    );
    assert_eq!(journal.snapshot()[0].state, TransactionState::Verified);
}

#[test]
fn undo_verifies_without_writing_when_the_service_already_holds_the_snapshot() {
    let adapter = ready_adapter();
    let (domain, journal) = recording_domain(adapter.clone());
    let preview = completed_preview(domain.preview(&profile()));
    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));
    adapter.set_service("DiagTrack", running_automatic());

    let restored = completed_change(domain.undo(&change.transaction_id));

    assert_eq!(restored.verified_state, running_automatic());
    assert_eq!(adapter.write_count(), 1);
    assert_eq!(journal.snapshot()[0].state, TransactionState::Restored);
}

#[test]
fn undo_reads_the_current_state_before_writing() {
    let adapter = ready_adapter();
    let (domain, _journal) = recording_domain(adapter.clone());
    let preview = completed_preview(domain.preview(&profile()));
    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));
    let events = adapter.events();
    lock(&events).clear();

    completed_change(domain.undo(&change.transaction_id));

    let log = lock(&events);
    let read = log
        .iter()
        .position(|event| event == "read:DiagTrack")
        .expect("undo reads the current state");
    let write = log
        .iter()
        .position(|event| event == "write:DiagTrack")
        .expect("undo writes the snapshot");
    assert!(read < write);
}

#[test]
fn undo_restores_delayed_auto_start_exactly() {
    let original =
        delayed_service_state(ServiceRunState::Running, ServiceStartType::Automatic, true);
    let adapter = FakeSystemAdapter::new().with_service("DiagTrack", original);
    let (domain, journal) = recording_domain(adapter.clone());
    let preview = completed_preview(domain.preview(&profile()));
    assert!(matches!(
        &preview.actions[0].observed,
        Observation::Observed { value } if value.delayed_start
    ));

    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));
    assert!(!change.verified_state.delayed_start);
    let restored = completed_change(domain.undo(&change.transaction_id));

    assert_eq!(restored.verified_state, original);
    assert_eq!(
        adapter
            .service_state("DiagTrack")
            .expect("service remains readable"),
        original
    );
    assert_eq!(journal.snapshot()[0].snapshot.state, Some(original));
}

#[test]
fn a_timed_out_write_is_recoverable_instead_of_permanently_blocked() {
    let adapter = ready_adapter();
    adapter.set_write_mode(WriteMode::Fail(WriteFailure::Timeout));
    let (domain, journal) = recording_domain(adapter.clone());
    let preview = completed_preview(domain.preview(&profile()));

    let CommandOutcome::RecoveryRequired(status) = domain.apply(&preview.id, &consent(&preview))
    else {
        panic!("expected recovery required");
    };
    let transaction_id = status.pending_transactions[0].transaction_id.clone();
    assert!(!status.writes_permitted);
    assert_eq!(
        journal.snapshot()[0].state,
        TransactionState::RecoveryRequired
    );

    adapter.set_write_mode(WriteMode::Apply);
    let restored = completed_change(domain.restore_transaction(&transaction_id));

    assert_eq!(restored.verified_state, running_automatic());
    assert!(completed_status(domain.recovery_status()).writes_permitted);
}

#[test]
fn apply_refuses_a_preview_with_more_than_one_applicable_action() {
    let actions = vec![
        catalog_action("diagtrack-privacy", "DiagTrack", target_state()),
        catalog_action("sysmain-service", "SysMain", target_state()),
    ];
    let adapter = FakeSystemAdapter::new()
        .with_service("DiagTrack", running_automatic())
        .with_service("SysMain", running_automatic());
    let journal = RecordingJournal::new(adapter.events());
    let domain =
        DomainService::with_catalog(adapter.clone(), journal.clone(), catalog_with(actions));
    let preview = completed_preview(domain.preview(&profile()));
    assert_eq!(preview.actions.len(), 2);

    let outcome = domain.apply(&preview.id, &consent(&preview));

    assert_eq!(
        outcome,
        CommandOutcome::Refused(Refusal::BatchApplyUnsupported {
            preview_id: preview.id.to_string(),
            actions: 2,
        })
    );
    assert_eq!(adapter.write_count(), 0);
    assert!(journal.snapshot().is_empty());
}

#[test]
fn apply_uses_the_applicable_action_when_the_first_action_is_a_no_op() {
    let actions = vec![
        catalog_action("diagtrack-privacy", "DiagTrack", target_state()),
        catalog_action("sysmain-service", "SysMain", target_state()),
    ];
    let adapter = FakeSystemAdapter::new()
        .with_service("DiagTrack", target_state())
        .with_service("SysMain", running_automatic());
    let journal = RecordingJournal::new(adapter.events());
    let domain =
        DomainService::with_catalog(adapter.clone(), journal.clone(), catalog_with(actions));
    let preview = completed_preview(domain.preview(&profile()));
    assert_eq!(preview.actions[0].disposition, Disposition::NoOp);
    assert_eq!(preview.actions[1].disposition, Disposition::Applicable);

    let change = completed_change(domain.apply(&preview.id, &consent(&preview)));

    assert_eq!(change.action_id.as_str(), "sysmain-service");
    assert_eq!(change.service, "SysMain");
    assert_eq!(adapter.write_count(), 1);
    assert_eq!(journal.snapshot().len(), 1);
}
