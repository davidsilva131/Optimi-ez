use std::fmt;
use std::fs::File;
use std::io::Write;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::id::{ActionId, TransactionId};
use crate::model::{Observation, ServiceSnapshot, ServiceState, WindowsIdentity, WindowsVersion};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Applying,
    Verified,
    RecoveryRequired,
    Restoring,
    Restored,
}

impl TransactionState {
    pub fn is_pending(&self) -> bool {
        matches!(
            self,
            Self::Applying | Self::RecoveryRequired | Self::Restoring
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRecord {
    pub transaction_id: TransactionId,
    pub action_id: ActionId,
    pub service: String,
    pub snapshot: ServiceSnapshot,
    pub target: ServiceState,
    pub windows_version: WindowsVersion,
    pub interactive_identity: WindowsIdentity,
    pub state: TransactionState,
    pub observed: Observation<ServiceState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalError {
    Io,
    Corrupt,
    MissingRecord,
    Busy,
}

impl fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io => formatter.write_str("journal input or output failed"),
            Self::Corrupt => formatter.write_str("journal contents are unreadable"),
            Self::MissingRecord => formatter.write_str("journal record is missing"),
            Self::Busy => formatter.write_str("journal is locked by another writer"),
        }
    }
}

impl std::error::Error for JournalError {}

pub struct JournalLock<'a> {
    _file: Option<File>,
    _scope: PhantomData<&'a ()>,
}

impl<'a> JournalLock<'a> {
    fn unlocked() -> Self {
        Self {
            _file: None,
            _scope: PhantomData,
        }
    }

    #[cfg(windows)]
    fn held(file: File) -> Self {
        Self {
            _file: Some(file),
            _scope: PhantomData,
        }
    }
}

pub trait JournalStore: Send + Sync {
    fn append(&self, record: JournalRecord) -> Result<(), JournalError>;
    fn update(&self, record: JournalRecord) -> Result<(), JournalError>;
    fn records(&self) -> Result<Vec<JournalRecord>, JournalError>;
    fn lock_exclusive(&self) -> Result<JournalLock<'_>, JournalError> {
        Ok(JournalLock::unlocked())
    }
}

pub struct MemoryJournal {
    records: Mutex<Vec<JournalRecord>>,
}

impl MemoryJournal {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
        }
    }
}

impl Default for MemoryJournal {
    fn default() -> Self {
        Self::new()
    }
}

impl JournalStore for MemoryJournal {
    fn append(&self, record: JournalRecord) -> Result<(), JournalError> {
        lock(&self.records).push(record);
        Ok(())
    }

    fn update(&self, record: JournalRecord) -> Result<(), JournalError> {
        let mut records = lock(&self.records);
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
        Ok(lock(&self.records).clone())
    }
}

pub struct FileJournal {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileJournal {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    fn read(&self) -> Result<Vec<JournalRecord>, JournalError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|_| JournalError::Corrupt),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(_) => Err(JournalError::Io),
        }
    }

    fn write(&self, records: &[JournalRecord]) -> Result<(), JournalError> {
        let parent = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|_| JournalError::Io)?;
        let data = serde_json::to_vec_pretty(records).map_err(|_| JournalError::Corrupt)?;
        let temporary = self.path.with_extension("tmp");
        let mut file = std::fs::File::create(&temporary).map_err(|_| JournalError::Io)?;
        file.write_all(&data).map_err(|_| JournalError::Io)?;
        file.sync_all().map_err(|_| JournalError::Io)?;
        drop(file);
        std::fs::rename(&temporary, &self.path).map_err(|_| JournalError::Io)
    }

    #[cfg(windows)]
    fn lock_path(&self) -> PathBuf {
        let mut path = self.path.as_os_str().to_owned();
        path.push(".lock");
        PathBuf::from(path)
    }
}

impl JournalStore for FileJournal {
    fn append(&self, record: JournalRecord) -> Result<(), JournalError> {
        let _guard = lock(&self.lock);
        let mut records = self.read()?;
        records.push(record);
        self.write(&records)
    }

    fn update(&self, record: JournalRecord) -> Result<(), JournalError> {
        let _guard = lock(&self.lock);
        let mut records = self.read()?;
        let Some(existing) = records
            .iter_mut()
            .find(|existing| existing.transaction_id == record.transaction_id)
        else {
            return Err(JournalError::MissingRecord);
        };
        *existing = record;
        self.write(&records)
    }

    fn records(&self) -> Result<Vec<JournalRecord>, JournalError> {
        let _guard = lock(&self.lock);
        self.read()
    }

    fn lock_exclusive(&self) -> Result<JournalLock<'_>, JournalError> {
        #[cfg(windows)]
        {
            open_exclusive(&self.lock_path()).map(JournalLock::held)
        }
        #[cfg(not(windows))]
        {
            Ok(JournalLock::unlocked())
        }
    }
}

#[cfg(windows)]
fn open_exclusive(path: &Path) -> Result<File, JournalError> {
    use std::os::windows::fs::OpenOptionsExt;

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|_| JournalError::Io)?;
    }
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
    {
        Ok(file) => Ok(file),
        Err(error) if matches!(error.raw_os_error(), Some(32) | Some(33)) => {
            Err(JournalError::Busy)
        }
        Err(_) => Err(JournalError::Io),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ServiceRunState, ServiceStartType, WindowsProductType};

    fn sample_record() -> JournalRecord {
        JournalRecord {
            transaction_id: TransactionId::parse("tx-1").expect("transaction id is valid"),
            action_id: ActionId::parse("diagtrack-privacy").expect("action id is valid"),
            service: "DiagTrack".to_owned(),
            snapshot: ServiceSnapshot {
                service: "DiagTrack".to_owned(),
                existed: true,
                state: Some(ServiceState {
                    run_state: ServiceRunState::Running,
                    start_type: ServiceStartType::Automatic,
                    delayed_start: true,
                }),
            },
            target: ServiceState {
                run_state: ServiceRunState::Stopped,
                start_type: ServiceStartType::Disabled,
                delayed_start: false,
            },
            windows_version: WindowsVersion {
                major: 10,
                minor: 0,
                build: 22631,
                product_type: WindowsProductType::Workstation,
            },
            interactive_identity: WindowsIdentity {
                account: "player".to_owned(),
                domain: None,
            },
            state: TransactionState::Applying,
            observed: Observation::observed(ServiceState {
                run_state: ServiceRunState::Running,
                start_type: ServiceStartType::Automatic,
                delayed_start: true,
            }),
        }
    }

    fn temporary_path() -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "optimi-journal-{}-{unique}.json",
            std::process::id()
        ))
    }

    #[test]
    fn file_journal_returns_no_records_when_the_file_is_absent() {
        let path = temporary_path();
        let journal = FileJournal::new(&path);

        assert!(journal.records().expect("absent journal reads").is_empty());
    }

    #[test]
    fn file_journal_persists_appends_and_updates() {
        let path = temporary_path();
        let journal = FileJournal::new(&path);
        let record = sample_record();
        journal.append(record.clone()).expect("record appends");

        let reloaded = FileJournal::new(&path).records().expect("journal reads");
        assert_eq!(reloaded, vec![record.clone()]);

        let mut updated = record;
        updated.state = TransactionState::Verified;
        journal.update(updated.clone()).expect("record updates");

        let reloaded = FileJournal::new(&path).records().expect("journal reads");
        assert_eq!(reloaded, vec![updated]);
        std::fs::remove_file(path).expect("temporary journal is removed");
    }

    #[cfg(windows)]
    #[test]
    fn file_journal_grants_one_exclusive_lock_at_a_time() {
        let path = temporary_path();
        let first = FileJournal::new(&path);
        let second = FileJournal::new(&path);

        let held = first.lock_exclusive().expect("first lock is acquired");
        assert_eq!(second.lock_exclusive().err(), Some(JournalError::Busy));

        drop(held);
        assert!(second.lock_exclusive().is_ok());

        std::fs::remove_file(first.lock_path()).expect("temporary lock is removed");
    }
}
