#![cfg(windows)]

use std::path::{Path, PathBuf};

use optimi_domain::adapter::SystemAdapter;
use optimi_domain::command::{CommandOutcome, Consent, DomainService};
use optimi_domain::id::{ProfileId, TransactionId};
use optimi_domain::journal::{FileJournal, JournalKey};
use optimi_domain::model::{Disposition, Observation, PlatformSupport};
use optimi_windows::WindowsSystemAdapter;

const VM_WRITE_GATE: &str = "OPTIMI_VM_WRITE_TEST";
const ELEVATED_INTEGRITY_SID: &str = "S-1-16-12288";

#[test]
#[ignore = "requires disposable Windows 11 VM"]
fn applies_and_restores_diagtrack_with_the_real_domain() {
    require_vm_write_gate();
    let adapter = WindowsSystemAdapter;
    require_supported_windows(&adapter);
    require_elevation();

    let journal_path = temporary_journal_path();
    let _journal_cleanup = JournalCleanup::new(&journal_path);
    let profile = ProfileId::parse("competitive").expect("embedded profile id is valid");
    let before = adapter
        .service_state("DiagTrack")
        .expect("DiagTrack must be readable before the test");
    let domain = DomainService::new(adapter, FileJournal::new(&journal_path, journal_key()));

    let preview = completed(domain.preview(&profile));
    assert_eq!(
        preview.platform,
        PlatformSupport::Supported,
        "the preview must be supported on this Windows build"
    );
    let action = preview
        .actions
        .iter()
        .find(|action| action.service == "DiagTrack")
        .expect("the competitive profile previews a DiagTrack action");
    assert_eq!(
        action.observed,
        Observation::observed(before),
        "the preview must observe the DiagTrack state the test read"
    );
    assert_eq!(
        action.disposition,
        Disposition::Applicable,
        "DiagTrack must start outside the catalog target, found {:?}; reset the VM snapshot",
        action.reason
    );
    let target = action.target;
    assert_ne!(
        before, target,
        "DiagTrack must start outside the catalog target; reset the VM snapshot"
    );
    println!("diagtrack before={before:?}");
    println!("diagtrack target={target:?}");

    let mut restore_on_exit = RestoreOnExit::new(journal_path.clone());
    let consent = Consent {
        preview_id: preview.id.clone(),
    };
    let change = completed(domain.apply(&preview.id, &consent));
    restore_on_exit.arm(change.transaction_id.clone());
    assert_eq!(change.service, "DiagTrack");
    assert_eq!(change.verified_state, target);
    let applied = adapter
        .service_state("DiagTrack")
        .expect("DiagTrack must be readable after apply");
    assert_eq!(
        applied, target,
        "apply must leave DiagTrack at the catalog target"
    );
    println!("diagtrack applied={applied:?}");

    let undone = completed(domain.undo(&change.transaction_id));
    restore_on_exit.disarm();
    assert_eq!(undone.transaction_id, change.transaction_id);
    assert_eq!(undone.verified_state, before);
    let restored = adapter
        .service_state("DiagTrack")
        .expect("DiagTrack must be readable after undo");
    assert_eq!(
        restored, before,
        "undo must restore the exact service snapshot"
    );
    assert_eq!(
        restored.delayed_start, before.delayed_start,
        "undo must restore the delayed auto-start flag"
    );
    println!("diagtrack restored={restored:?}");
}

fn require_vm_write_gate() {
    let enabled = std::env::var(VM_WRITE_GATE).unwrap_or_default();
    assert_eq!(
        enabled, "1",
        "refusing to run: set {VM_WRITE_GATE}=1 only on a disposable Windows 11 VM"
    );
}

fn require_supported_windows(adapter: &impl SystemAdapter) {
    let version = adapter
        .windows_version()
        .expect("refusing to run: the Windows version must be readable");
    assert!(
        version.supports_current_release(),
        "refusing to run: this test requires Windows 11 Workstation build 22000 or later, found {version:?}"
    );
}

fn require_elevation() {
    let groups = std::process::Command::new("whoami")
        .arg("/groups")
        .output()
        .expect("refusing to run: the process groups must be readable");
    assert!(
        String::from_utf8_lossy(&groups.stdout).contains(ELEVATED_INTEGRITY_SID),
        "refusing to run: an elevated shell is required to change service configuration on a disposable Windows 11 VM"
    );
}

fn completed<T: std::fmt::Debug>(outcome: CommandOutcome<T>) -> T {
    match outcome {
        CommandOutcome::Completed(value) => value,
        other => panic!("expected a completed outcome, got {other:?}"),
    }
}

fn journal_key() -> JournalKey {
    JournalKey::new([0x6d; JournalKey::LENGTH])
}

fn temporary_journal_path() -> PathBuf {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "optimi-vm-write-{}-{unique}.json",
        std::process::id()
    ))
}

struct JournalCleanup {
    path: PathBuf,
    lock_path: PathBuf,
}

impl JournalCleanup {
    fn new(path: &Path) -> Self {
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".lock");
        Self {
            path: path.to_path_buf(),
            lock_path: PathBuf::from(lock_path),
        }
    }
}

impl Drop for JournalCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("tmp"));
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

struct RestoreOnExit {
    journal_path: PathBuf,
    transaction_id: Option<TransactionId>,
}

impl RestoreOnExit {
    fn new(journal_path: PathBuf) -> Self {
        Self {
            journal_path,
            transaction_id: None,
        }
    }

    fn arm(&mut self, transaction_id: TransactionId) {
        self.transaction_id = Some(transaction_id);
    }

    fn disarm(&mut self) {
        self.transaction_id = None;
    }
}

impl Drop for RestoreOnExit {
    fn drop(&mut self) {
        let journal = FileJournal::new(&self.journal_path, journal_key());
        let domain = DomainService::new(WindowsSystemAdapter, journal);
        match self.transaction_id.take() {
            Some(transaction_id) => {
                let _ = domain.undo(&transaction_id);
            }
            None => {
                let _ = domain.restore_all();
            }
        }
    }
}
