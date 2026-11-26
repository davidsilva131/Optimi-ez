use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use optimi_domain::adapter::SystemAdapter;
use optimi_domain::command::{CommandFailure, CommandOutcome, DomainService};
use optimi_domain::id::{ActionId, TransactionId};
use optimi_domain::journal::{
    FileJournal, JournalKey, JournalRecord, JournalStore, TransactionState,
};
use optimi_domain::model::{
    ActivePowerPlan, AntiCheatState, Observation, ReadFailure, ServiceRunState, ServiceSnapshot,
    ServiceStartType, ServiceState, WindowsIdentity, WindowsProductType, WindowsVersion,
    WriteFailure,
};

#[derive(Clone)]
struct FakeAdapter {
    state: Arc<Mutex<ServiceState>>,
    writes: Arc<Mutex<Vec<ServiceState>>>,
}

impl FakeAdapter {
    fn at(state: ServiceState) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
            writes: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn writes(&self) -> usize {
        lock(&self.writes).len()
    }
}

impl SystemAdapter for FakeAdapter {
    fn windows_version(&self) -> Result<WindowsVersion, ReadFailure> {
        Ok(windows_version())
    }

    fn interactive_identity(&self) -> Result<WindowsIdentity, ReadFailure> {
        Ok(identity())
    }

    fn service_state(&self, _: &str) -> Result<ServiceState, ReadFailure> {
        Ok(*lock(&self.state))
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

    fn write_service_state(&self, _: &str, target: ServiceState) -> Result<(), WriteFailure> {
        *lock(&self.state) = target;
        lock(&self.writes).push(target);
        Ok(())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn running_automatic() -> ServiceState {
    ServiceState {
        run_state: ServiceRunState::Running,
        start_type: ServiceStartType::Automatic,
        delayed_start: true,
    }
}

fn stopped_disabled() -> ServiceState {
    ServiceState {
        run_state: ServiceRunState::Stopped,
        start_type: ServiceStartType::Disabled,
        delayed_start: false,
    }
}

fn windows_version() -> WindowsVersion {
    WindowsVersion {
        major: 10,
        minor: 0,
        build: 22631,
        product_type: WindowsProductType::Workstation,
    }
}

fn identity() -> WindowsIdentity {
    WindowsIdentity {
        account: "player".to_owned(),
        domain: Some("TEST".to_owned()),
    }
}

fn transaction() -> TransactionId {
    TransactionId::parse("tx-1").expect("transaction id is valid")
}

fn record() -> JournalRecord {
    JournalRecord {
        transaction_id: transaction(),
        action_id: ActionId::parse("diagtrack-privacy").expect("action id is valid"),
        service: "DiagTrack".to_owned(),
        snapshot: ServiceSnapshot {
            service: "DiagTrack".to_owned(),
            existed: true,
            state: Some(running_automatic()),
        },
        target: stopped_disabled(),
        windows_version: windows_version(),
        interactive_identity: identity(),
        state: TransactionState::Applying,
        observed: Observation::observed(running_automatic()),
    }
}

fn key() -> JournalKey {
    JournalKey::new([0x3c; JournalKey::LENGTH])
}

struct Scratch {
    directory: PathBuf,
    journal: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "optimi-integrity-{name}-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("scratch directory is created");
        let journal = directory.join("journal.json");
        Self { directory, journal }
    }

    fn seed(&self) {
        FileJournal::new(&self.journal, key())
            .append(record())
            .expect("signed record is written");
    }

    fn edit(&self, pointer: &str, value: serde_json::Value) {
        let mut document: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&self.journal).expect("journal is readable"),
        )
        .expect("journal is json");
        *document
            .pointer_mut(pointer)
            .expect("tampered field exists") = value;
        std::fs::write(
            &self.journal,
            serde_json::to_vec(&document).expect("journal encodes"),
        )
        .expect("tampered journal is written");
    }

    fn strip_mac(&self) {
        let mut document: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&self.journal).expect("journal is readable"),
        )
        .expect("journal is json");
        document[0]
            .as_object_mut()
            .expect("stored record is an object")
            .remove("mac");
        std::fs::write(
            &self.journal,
            serde_json::to_vec(&document).expect("journal encodes"),
        )
        .expect("journal is written");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn restore(
    scratch: &Scratch,
    adapter: FakeAdapter,
) -> (CommandOutcome<optimi_domain::model::VerifiedChange>, usize) {
    let domain = DomainService::new(adapter.clone(), FileJournal::new(&scratch.journal, key()));
    let outcome = domain.restore_transaction(&transaction());
    assert!(Path::new(&scratch.journal).exists());
    (outcome, adapter.writes())
}

#[test]
fn a_tampered_snapshot_state_invalidates_the_journal_and_writes_nothing() {
    let scratch = Scratch::new("snapshot");
    scratch.seed();
    scratch.edit("/0/record/snapshot/state/run_state", "stopped".into());
    let before = std::fs::read(&scratch.journal).expect("journal is readable");

    let (outcome, writes) = restore(&scratch, FakeAdapter::at(stopped_disabled()));

    assert_eq!(outcome, CommandOutcome::Failed(CommandFailure::JournalRead));
    assert_eq!(writes, 0);
    assert_eq!(
        std::fs::read(&scratch.journal).expect("journal is readable"),
        before
    );
}

#[test]
fn a_tampered_observed_state_invalidates_the_journal_and_writes_nothing() {
    let scratch = Scratch::new("observed");
    scratch.seed();
    scratch.edit("/0/record/observed/value/start_type", "disabled".into());

    let (outcome, writes) = restore(&scratch, FakeAdapter::at(stopped_disabled()));

    assert_eq!(outcome, CommandOutcome::Failed(CommandFailure::JournalRead));
    assert_eq!(writes, 0);
}

#[test]
fn a_record_without_a_mac_invalidates_the_journal_and_writes_nothing() {
    let scratch = Scratch::new("missing-mac");
    scratch.seed();
    scratch.strip_mac();

    let (outcome, writes) = restore(&scratch, FakeAdapter::at(stopped_disabled()));

    assert_eq!(outcome, CommandOutcome::Failed(CommandFailure::JournalRead));
    assert_eq!(writes, 0);
}

#[test]
fn a_valid_signed_journal_restores_the_recorded_snapshot() {
    let scratch = Scratch::new("valid");
    scratch.seed();

    let (outcome, writes) = restore(&scratch, FakeAdapter::at(stopped_disabled()));

    let CommandOutcome::Completed(change) = outcome else {
        panic!("one signed record must restore, found {outcome:?}");
    };
    assert_eq!(change.transaction_id, transaction());
    assert_eq!(change.verified_state, running_automatic());
    assert_eq!(writes, 1);
    let records = FileJournal::new(&scratch.journal, key())
        .records()
        .expect("restored journal verifies");
    assert_eq!(records[0].state, TransactionState::Restored);
}
