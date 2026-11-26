use std::fmt;
use std::fs::File;
use std::io::Write;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::id::{ActionId, TransactionId};
use crate::model::{Observation, ServiceSnapshot, ServiceState, WindowsIdentity, WindowsVersion};

const RECORD_MAC_DOMAIN: &[u8] = b"optimi-journal-record-v1";
const HASH_LENGTH: usize = 32;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct JournalKey([u8; HASH_LENGTH]);

impl JournalKey {
    pub const LENGTH: usize = HASH_LENGTH;

    pub fn new(bytes: [u8; HASH_LENGTH]) -> Self {
        Self(bytes)
    }

    fn hmac(&self) -> Result<HmacSha256, JournalError> {
        HmacSha256::new_from_slice(&self.0).map_err(|_| JournalError::Io)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionState {
    Applying,
    Verified,
    RecoveryRequired,
    Restoring,
    Restored,
    Abandoned,
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
#[serde(deny_unknown_fields)]
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
    ReadOnly,
}

impl fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io => formatter.write_str("journal input or output failed"),
            Self::Corrupt => formatter.write_str("journal contents are unreadable"),
            Self::MissingRecord => formatter.write_str("journal record is missing"),
            Self::Busy => formatter.write_str("journal is locked by another writer"),
            Self::ReadOnly => {
                formatter.write_str("journal is read-only outside the elevated worker")
            }
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRecord {
    record: JournalRecord,
    mac: Option<String>,
}

pub struct FileJournal {
    path: PathBuf,
    key: JournalKey,
    lock: Mutex<()>,
}

impl FileJournal {
    pub fn new(path: impl Into<PathBuf>, key: JournalKey) -> Self {
        Self {
            path: path.into(),
            key,
            lock: Mutex::new(()),
        }
    }

    fn read(&self) -> Result<Vec<JournalRecord>, JournalError> {
        read_signed_records(&self.path, &self.key)
    }

    fn write(&self, records: &[JournalRecord]) -> Result<(), JournalError> {
        let stored: Vec<StoredRecord> = records
            .iter()
            .map(|record| sign_record(&self.key, record))
            .collect::<Result<_, _>>()?;
        let data = serde_json::to_vec_pretty(&stored).map_err(|_| JournalError::Corrupt)?;
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

pub struct ReadOnlyJournal {
    path: PathBuf,
}

impl ReadOnlyJournal {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl JournalStore for ReadOnlyJournal {
    fn append(&self, _: JournalRecord) -> Result<(), JournalError> {
        Err(JournalError::ReadOnly)
    }

    fn update(&self, _: JournalRecord) -> Result<(), JournalError> {
        Err(JournalError::ReadOnly)
    }

    fn records(&self) -> Result<Vec<JournalRecord>, JournalError> {
        read_display_records(&self.path)
    }

    fn lock_exclusive(&self) -> Result<JournalLock<'_>, JournalError> {
        Err(JournalError::ReadOnly)
    }
}

fn read_signed_records(path: &Path, key: &JournalKey) -> Result<Vec<JournalRecord>, JournalError> {
    read_stored_records(path)?
        .into_iter()
        .map(|stored| verify_record(key, stored))
        .collect()
}

fn read_display_records(path: &Path) -> Result<Vec<JournalRecord>, JournalError> {
    Ok(read_stored_records(path)?
        .into_iter()
        .map(|stored| stored.record)
        .collect())
}

fn read_stored_records(path: &Path) -> Result<Vec<StoredRecord>, JournalError> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|_| JournalError::Corrupt),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err(JournalError::Io),
    }
}

fn sign_record(key: &JournalKey, record: &JournalRecord) -> Result<StoredRecord, JournalError> {
    let mut mac = key.hmac()?;
    mac.update(RECORD_MAC_DOMAIN);
    mac.update(&serde_json::to_vec(record).map_err(|_| JournalError::Corrupt)?);
    Ok(StoredRecord {
        record: record.clone(),
        mac: Some(to_hex(&mac.finalize().into_bytes())),
    })
}

fn verify_record(key: &JournalKey, stored: StoredRecord) -> Result<JournalRecord, JournalError> {
    let mac = stored.mac.as_deref().ok_or(JournalError::Corrupt)?;
    let presented = from_hex(mac).ok_or(JournalError::Corrupt)?;
    let mut expected = key.hmac()?;
    expected.update(RECORD_MAC_DOMAIN);
    expected.update(&serde_json::to_vec(&stored.record).map_err(|_| JournalError::Corrupt)?);
    expected
        .verify_slice(&presented)
        .map_err(|_| JournalError::Corrupt)?;
    Ok(stored.record)
}

fn to_hex(bytes: &[u8]) -> String {
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        value.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    value
}

fn from_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            Some((high << 4) | low)
        })
        .collect()
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
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

    fn key() -> JournalKey {
        JournalKey::new([0x5a; JournalKey::LENGTH])
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
        let journal = FileJournal::new(&path, key());

        assert!(journal.records().expect("absent journal reads").is_empty());
    }

    #[test]
    fn file_journal_persists_appends_and_updates() {
        let path = temporary_path();
        let journal = FileJournal::new(&path, key());
        let record = sample_record();
        journal.append(record.clone()).expect("record appends");

        let reloaded = FileJournal::new(&path, key())
            .records()
            .expect("journal reads");
        assert_eq!(reloaded, vec![record.clone()]);

        let mut updated = record;
        updated.state = TransactionState::Verified;
        journal.update(updated.clone()).expect("record updates");

        let reloaded = FileJournal::new(&path, key())
            .records()
            .expect("journal reads");
        assert_eq!(reloaded, vec![updated]);
        std::fs::remove_file(path).expect("temporary journal is removed");
    }

    #[test]
    fn read_only_journal_reads_records_and_refuses_every_write() {
        let path = temporary_path();
        let writable = FileJournal::new(&path, key());
        let record = sample_record();
        writable.append(record.clone()).expect("record appends");

        let read_only = ReadOnlyJournal::open(&path);
        assert_eq!(
            read_only.records().expect("journal reads"),
            vec![record.clone()]
        );
        assert_eq!(
            read_only.append(record.clone()).err(),
            Some(JournalError::ReadOnly)
        );
        assert_eq!(read_only.update(record).err(), Some(JournalError::ReadOnly));
        assert_eq!(
            read_only.lock_exclusive().err(),
            Some(JournalError::ReadOnly)
        );
        std::fs::remove_file(path).expect("temporary journal is removed");
    }

    #[cfg(windows)]
    #[test]
    fn file_journal_grants_one_exclusive_lock_at_a_time() {
        let path = temporary_path();
        let first = FileJournal::new(&path, key());
        let second = FileJournal::new(&path, key());

        let held = first.lock_exclusive().expect("first lock is acquired");
        assert_eq!(second.lock_exclusive().err(), Some(JournalError::Busy));

        drop(held);
        assert!(second.lock_exclusive().is_ok());

        let _ = std::fs::remove_file(first.lock_path());
    }

    #[test]
    fn read_only_journal_returns_records_without_verifying_their_mac() {
        let path = temporary_path();
        let record = sample_record();
        FileJournal::new(&path, key())
            .append(record.clone())
            .expect("record appends");
        let corrupted = std::fs::read_to_string(&path)
            .expect("journal is readable")
            .replace("\"mac\": \"", "\"mac\": \"00");
        std::fs::write(&path, corrupted).expect("journal is corrupted");

        let read_only = ReadOnlyJournal::open(&path);
        assert_eq!(
            read_only
                .records()
                .expect("unprivileged reads skip the mac"),
            vec![record]
        );
        assert_eq!(
            FileJournal::new(&path, key()).records().err(),
            Some(JournalError::Corrupt)
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_record_without_a_mac_is_rejected_by_the_worker() {
        let path = temporary_path();
        let record = sample_record();
        FileJournal::new(&path, key())
            .append(record.clone())
            .expect("record appends");
        let stored: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("journal is readable"))
                .expect("journal is json");
        let mut stored = stored.as_array().expect("journal is an array").clone();
        stored[0]
            .as_object_mut()
            .expect("stored record is an object")
            .remove("mac");
        std::fs::write(&path, serde_json::to_vec(&stored).expect("journal encodes"))
            .expect("journal is rewritten");

        assert_eq!(
            FileJournal::new(&path, key()).records().err(),
            Some(JournalError::Corrupt)
        );
        assert_eq!(
            ReadOnlyJournal::open(&path)
                .records()
                .expect("unprivileged reads do not validate"),
            vec![record]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_journal_signed_with_another_key_is_rejected() {
        let path = temporary_path();
        FileJournal::new(&path, key())
            .append(sample_record())
            .expect("record appends");

        assert_eq!(
            FileJournal::new(&path, JournalKey::new([0x11; JournalKey::LENGTH]))
                .records()
                .err(),
            Some(JournalError::Corrupt)
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn hex_round_trips_every_byte_value() {
        let bytes: Vec<u8> = (0..=255).collect();

        assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }
}
