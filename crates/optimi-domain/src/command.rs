use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Serialize;

use crate::adapter::SystemAdapter;
use crate::catalog::{Catalog, CatalogAction};
use crate::id::{PreviewId, ProfileId, TransactionId};
use crate::journal::{JournalError, JournalLock, JournalRecord, JournalStore, TransactionState};
use crate::model::{
    ActionPreview, Disposition, DispositionReason, Gate, Observation, PlatformSupport, Preview,
    ReadFailure, ServiceRunState, ServiceSnapshot, ServiceStartType, ServiceState, VerifiedChange,
    WindowsVersion,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Consent {
    pub preview_id: PreviewId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Refusal {
    UnknownProfile {
        profile_id: String,
    },
    InvalidPreviewId {
        value: String,
    },
    InvalidTransactionId {
        value: String,
    },
    ConsentMismatch,
    PreviewNotCurrent {
        preview_id: String,
    },
    PreviewStateChanged {
        preview_id: String,
    },
    PreviewStateUnavailable {
        preview_id: String,
    },
    NoApplicableAction {
        preview_id: String,
    },
    BatchApplyUnsupported {
        preview_id: String,
        actions: usize,
    },
    ServiceStateNotSettled {
        service: String,
        state: ServiceRunState,
    },
    GateBlocked {
        gate: Gate,
    },
    GateReadFailed {
        gate: Gate,
    },
    WriteInProgress,
    UnknownTransaction {
        transaction_id: String,
    },
    TransactionNotRestorable {
        transaction_id: String,
        state: TransactionState,
    },
    AlreadyRestored {
        transaction_id: String,
    },
    RestoreConflict {
        transaction_id: String,
        expected: ServiceState,
        current: ServiceState,
    },
    RestoreStateUnavailable {
        transaction_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandFailure {
    JournalWrite,
    JournalRead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteBlocker {
    PendingRecovery,
    JournalUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingTransaction {
    pub transaction_id: TransactionId,
    pub service: String,
    pub state: TransactionState,
    pub snapshot: ServiceSnapshot,
    pub observed: Observation<ServiceState>,
}

impl From<&JournalRecord> for PendingTransaction {
    fn from(record: &JournalRecord) -> Self {
        Self {
            transaction_id: record.transaction_id.clone(),
            service: record.service.clone(),
            state: record.state,
            snapshot: record.snapshot.clone(),
            observed: record.observed.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoveryStatus {
    pub pending_transactions: Vec<PendingTransaction>,
    pub writes_permitted: bool,
    pub blocker: Option<WriteBlocker>,
}

impl RecoveryStatus {
    fn pending(record: &JournalRecord) -> Self {
        Self {
            pending_transactions: vec![PendingTransaction::from(record)],
            writes_permitted: false,
            blocker: Some(WriteBlocker::PendingRecovery),
        }
    }

    fn journal_unavailable() -> Self {
        Self {
            pending_transactions: Vec::new(),
            writes_permitted: false,
            blocker: Some(WriteBlocker::JournalUnavailable),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", content = "payload", rename_all = "snake_case")]
pub enum CommandOutcome<T> {
    Completed(T),
    Refused(Refusal),
    RecoveryRequired(RecoveryStatus),
    Failed(CommandFailure),
}

#[derive(Clone)]
struct IssuedPreview {
    preview: Preview,
}

struct WriteGate {
    in_flight: bool,
}

struct WriteGuard<'a> {
    gate: &'a Mutex<WriteGate>,
    journal: Option<JournalLock<'a>>,
}

impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        drop(self.journal.take());
        lock(self.gate).in_flight = false;
    }
}

enum WriteStart<'a> {
    Acquired(WriteGuard<'a>),
    Busy,
    Unavailable,
}

pub struct DomainService<A, J> {
    adapter: A,
    journal: J,
    catalog: Catalog,
    preview_sequence: AtomicU64,
    transaction_sequence: AtomicU64,
    previews: Mutex<Vec<IssuedPreview>>,
    write_gate: Mutex<WriteGate>,
}

impl<A: SystemAdapter, J: JournalStore> DomainService<A, J> {
    pub fn new(adapter: A, journal: J) -> Self {
        Self::from_parts(adapter, journal, Catalog::embedded())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_catalog(adapter: A, journal: J, catalog: Catalog) -> Self {
        Self::from_parts(adapter, journal, catalog)
    }

    fn from_parts(adapter: A, journal: J, catalog: Catalog) -> Self {
        Self {
            adapter,
            journal,
            catalog,
            preview_sequence: AtomicU64::new(0),
            transaction_sequence: AtomicU64::new(0),
            previews: Mutex::new(Vec::new()),
            write_gate: Mutex::new(WriteGate { in_flight: false }),
        }
    }

    pub fn preview(&self, profile_id: &ProfileId) -> CommandOutcome<Preview> {
        let Some(profile) = self.catalog.profile(profile_id) else {
            return CommandOutcome::Refused(Refusal::UnknownProfile {
                profile_id: profile_id.to_string(),
            });
        };
        let windows_version = match self.adapter.windows_version() {
            Ok(version) => Observation::observed(version),
            Err(failure) => Observation::unavailable(failure),
        };
        let platform = platform_for(&windows_version);
        let actions = profile
            .action_ids
            .iter()
            .map(|action_id| {
                let action = self
                    .catalog
                    .action(action_id)
                    .expect("catalog profile references an embedded action");
                self.action_preview(action, platform)
            })
            .collect();
        let preview = Preview {
            id: self.next_preview_id(),
            profile_id: profile_id.clone(),
            catalog_version: self.catalog.version.to_owned(),
            platform,
            windows_version,
            interactive_identity: match self.adapter.interactive_identity() {
                Ok(identity) => Observation::observed(identity),
                Err(failure) => Observation::unavailable(failure),
            },
            active_power_plan: match self.adapter.active_power_plan() {
                Ok(plan) => Observation::observed(plan),
                Err(failure) => Observation::unavailable(failure),
            },
            actions,
        };
        let mut previews = lock(&self.previews);
        previews.retain(|issued| issued.preview.profile_id != preview.profile_id);
        previews.push(IssuedPreview {
            preview: preview.clone(),
        });
        CommandOutcome::Completed(preview)
    }

    pub fn apply(
        &self,
        preview_id: &PreviewId,
        consent: &Consent,
    ) -> CommandOutcome<VerifiedChange> {
        if &consent.preview_id != preview_id {
            return CommandOutcome::Refused(Refusal::ConsentMismatch);
        }
        let _guard = match self.begin_write() {
            WriteStart::Acquired(guard) => guard,
            WriteStart::Busy => return CommandOutcome::Refused(Refusal::WriteInProgress),
            WriteStart::Unavailable => {
                return CommandOutcome::Failed(CommandFailure::JournalWrite);
            }
        };
        let records = match self.journal.records() {
            Ok(records) => records,
            Err(_) => {
                return CommandOutcome::RecoveryRequired(RecoveryStatus::journal_unavailable());
            }
        };
        if records.iter().any(|record| record.state.is_pending()) {
            return CommandOutcome::RecoveryRequired(recovery_status(&records));
        }
        let Some(issued) = self.issued_preview(preview_id) else {
            return CommandOutcome::Refused(Refusal::PreviewNotCurrent {
                preview_id: preview_id.to_string(),
            });
        };
        if issued.preview.actions.iter().any(|action| {
            matches!(
                action.disposition,
                Disposition::Blocked | Disposition::Unknown
            )
        }) {
            return CommandOutcome::Refused(Refusal::NoApplicableAction {
                preview_id: preview_id.to_string(),
            });
        }
        let applicable: Vec<&ActionPreview> = issued
            .preview
            .actions
            .iter()
            .filter(|action| action.disposition == Disposition::Applicable)
            .collect();
        let Some(action_preview) = applicable.first().copied() else {
            return CommandOutcome::Refused(Refusal::NoApplicableAction {
                preview_id: preview_id.to_string(),
            });
        };
        if applicable.len() > 1 {
            return CommandOutcome::Refused(Refusal::BatchApplyUnsupported {
                preview_id: preview_id.to_string(),
                actions: applicable.len(),
            });
        }
        let action = self
            .catalog
            .action(&action_preview.action_id)
            .expect("preview references an embedded action")
            .clone();
        let windows_version = match self.check_platform() {
            Ok(version) => version,
            Err(refusal) => return CommandOutcome::Refused(refusal),
        };
        if let Err(refusal) = self.check_anti_cheat() {
            return CommandOutcome::Refused(refusal);
        }
        let interactive_identity = match self.adapter.interactive_identity() {
            Ok(identity) => identity,
            Err(_) => {
                return CommandOutcome::Refused(Refusal::PreviewStateUnavailable {
                    preview_id: preview_id.to_string(),
                });
            }
        };
        let version_matches = matches!(
            &issued.preview.windows_version,
            Observation::Observed { value } if value == &windows_version
        );
        let identity_matches = matches!(
            &issued.preview.interactive_identity,
            Observation::Observed { value } if value == &interactive_identity
        );
        if !version_matches || !identity_matches {
            return CommandOutcome::Refused(Refusal::PreviewStateChanged {
                preview_id: preview_id.to_string(),
            });
        }
        let observed = match self.adapter.service_state(&action.service) {
            Ok(state) => state,
            Err(ReadFailure::NotFound) => {
                return CommandOutcome::Refused(Refusal::PreviewStateChanged {
                    preview_id: preview_id.to_string(),
                });
            }
            Err(_) => {
                return CommandOutcome::Refused(Refusal::PreviewStateUnavailable {
                    preview_id: preview_id.to_string(),
                });
            }
        };
        let observed_matches = matches!(
            &action_preview.observed,
            Observation::Observed { value } if *value == observed
        );
        if !observed_matches {
            return CommandOutcome::Refused(Refusal::PreviewStateChanged {
                preview_id: preview_id.to_string(),
            });
        }
        if !observed.run_state.is_settled() {
            return CommandOutcome::Refused(Refusal::ServiceStateNotSettled {
                service: action.service.clone(),
                state: observed.run_state,
            });
        }

        let transaction_id = self.next_transaction_id(&records);
        let mut record = JournalRecord {
            transaction_id: transaction_id.clone(),
            action_id: action.id.clone(),
            service: action.service.clone(),
            snapshot: ServiceSnapshot {
                service: action.service.clone(),
                existed: true,
                state: Some(observed),
            },
            target: action.target,
            windows_version,
            interactive_identity,
            state: TransactionState::Applying,
            observed: Observation::observed(observed),
        };
        if self.journal.append(record.clone()).is_err() {
            return CommandOutcome::Failed(CommandFailure::JournalWrite);
        }
        self.consume_preview(preview_id);

        if self
            .adapter
            .write_service_state(&action.service, action.target)
            .is_err()
        {
            record.state = TransactionState::RecoveryRequired;
            record.observed = self.observe_service(&action.service);
            return self.recovery_required(record);
        }

        let verification = self.observe_service(&action.service);
        let verified = matches!(
            &verification,
            Observation::Observed { value } if *value == action.target
        );
        record.observed = verification;
        if !verified {
            record.state = TransactionState::RecoveryRequired;
            return self.recovery_required(record);
        }
        let change = VerifiedChange {
            transaction_id: transaction_id.clone(),
            action_id: action.id.clone(),
            service: action.service.clone(),
            verified_state: action.target,
        };
        record.state = TransactionState::Verified;
        if self.journal.update(record.clone()).is_err() {
            record.state = TransactionState::RecoveryRequired;
            return self.recovery_required(record);
        }
        CommandOutcome::Completed(change)
    }

    pub fn undo(&self, transaction_id: &TransactionId) -> CommandOutcome<VerifiedChange> {
        let _guard = match self.begin_write() {
            WriteStart::Acquired(guard) => guard,
            WriteStart::Busy => return CommandOutcome::Refused(Refusal::WriteInProgress),
            WriteStart::Unavailable => {
                return CommandOutcome::Failed(CommandFailure::JournalWrite);
            }
        };
        self.restore_locked(transaction_id)
    }

    pub fn restore_transaction(
        &self,
        transaction_id: &TransactionId,
    ) -> CommandOutcome<VerifiedChange> {
        let _guard = match self.begin_write() {
            WriteStart::Acquired(guard) => guard,
            WriteStart::Busy => return CommandOutcome::Refused(Refusal::WriteInProgress),
            WriteStart::Unavailable => {
                return CommandOutcome::Failed(CommandFailure::JournalWrite);
            }
        };
        self.restore_locked(transaction_id)
    }

    pub fn restore_all(&self) -> CommandOutcome<Vec<VerifiedChange>> {
        let _guard = match self.begin_write() {
            WriteStart::Acquired(guard) => guard,
            WriteStart::Busy => return CommandOutcome::Refused(Refusal::WriteInProgress),
            WriteStart::Unavailable => {
                return CommandOutcome::Failed(CommandFailure::JournalWrite);
            }
        };
        let records = match self.journal.records() {
            Ok(records) => records,
            Err(_) => return CommandOutcome::Failed(CommandFailure::JournalRead),
        };
        let pending: Vec<TransactionId> = records
            .iter()
            .filter(|record| record.state.is_pending())
            .map(|record| record.transaction_id.clone())
            .collect();
        let mut changes = Vec::with_capacity(pending.len());
        for transaction_id in pending {
            match self.restore_locked(&transaction_id) {
                CommandOutcome::Completed(change) => changes.push(change),
                CommandOutcome::Refused(refusal) => return CommandOutcome::Refused(refusal),
                CommandOutcome::RecoveryRequired(status) => {
                    return CommandOutcome::RecoveryRequired(status);
                }
                CommandOutcome::Failed(failure) => return CommandOutcome::Failed(failure),
            }
        }
        CommandOutcome::Completed(changes)
    }

    fn restore_locked(&self, transaction_id: &TransactionId) -> CommandOutcome<VerifiedChange> {
        let records = match self.journal.records() {
            Ok(records) => records,
            Err(_) => return CommandOutcome::Failed(CommandFailure::JournalRead),
        };
        let Some(mut record) = records
            .into_iter()
            .find(|record| &record.transaction_id == transaction_id)
        else {
            return CommandOutcome::Refused(Refusal::UnknownTransaction {
                transaction_id: transaction_id.to_string(),
            });
        };
        if record.state == TransactionState::Restored {
            return CommandOutcome::Refused(Refusal::AlreadyRestored {
                transaction_id: transaction_id.to_string(),
            });
        }
        let Some(snapshot_state) = record.snapshot.state else {
            return CommandOutcome::Refused(Refusal::TransactionNotRestorable {
                transaction_id: transaction_id.to_string(),
                state: record.state,
            });
        };
        if !snapshot_state.run_state.is_settled() {
            return CommandOutcome::Refused(Refusal::TransactionNotRestorable {
                transaction_id: transaction_id.to_string(),
                state: record.state,
            });
        }
        if let Err(refusal) = self.check_platform() {
            return CommandOutcome::Refused(refusal);
        }
        if let Err(refusal) = self.check_anti_cheat() {
            return CommandOutcome::Refused(refusal);
        }
        let current = match self.adapter.service_state(&record.service) {
            Ok(state) => state,
            Err(_) => {
                return CommandOutcome::Refused(Refusal::RestoreStateUnavailable {
                    transaction_id: transaction_id.to_string(),
                });
            }
        };
        let change = VerifiedChange {
            transaction_id: record.transaction_id.clone(),
            action_id: record.action_id.clone(),
            service: record.service.clone(),
            verified_state: snapshot_state,
        };
        if current == snapshot_state {
            record.observed = Observation::observed(current);
            record.state = TransactionState::Restored;
            if self.journal.update(record).is_err() {
                return CommandOutcome::Failed(CommandFailure::JournalWrite);
            }
            return CommandOutcome::Completed(change);
        }
        let attributable = current == record.target
            || matches!(&record.observed, Observation::Observed { value } if *value == current);
        if !attributable {
            return CommandOutcome::Refused(Refusal::RestoreConflict {
                transaction_id: transaction_id.to_string(),
                expected: snapshot_state,
                current,
            });
        }
        record.state = TransactionState::Restoring;
        if self.journal.update(record.clone()).is_err() {
            return CommandOutcome::Failed(CommandFailure::JournalWrite);
        }
        if self
            .adapter
            .write_service_state(&record.service, snapshot_state)
            .is_err()
        {
            record.state = TransactionState::RecoveryRequired;
            record.observed = self.observe_service(&record.service);
            return self.recovery_required(record);
        }
        let verification = self.observe_service(&record.service);
        let restored = matches!(
            &verification,
            Observation::Observed { value } if *value == snapshot_state
        );
        record.observed = verification;
        if !restored {
            record.state = TransactionState::RecoveryRequired;
            return self.recovery_required(record);
        }
        record.state = TransactionState::Restored;
        if self.journal.update(record.clone()).is_err() {
            record.state = TransactionState::RecoveryRequired;
            return self.recovery_required(record);
        }
        CommandOutcome::Completed(change)
    }

    pub fn recovery_status(&self) -> CommandOutcome<RecoveryStatus> {
        match self.journal.records() {
            Ok(records) => CommandOutcome::Completed(recovery_status(&records)),
            Err(_) => CommandOutcome::Completed(RecoveryStatus::journal_unavailable()),
        }
    }

    fn action_preview(&self, action: &CatalogAction, platform: PlatformSupport) -> ActionPreview {
        let observed = match self.adapter.service_state(&action.service) {
            Ok(state) => Observation::observed(state),
            Err(failure) => Observation::unavailable(failure),
        };
        let (disposition, reason) = disposition_for(platform, &observed, action.target);
        ActionPreview {
            action_id: action.id.clone(),
            service: action.service.clone(),
            risk: action.risk,
            evidence: action.evidence.clone(),
            target: action.target,
            observed,
            disposition,
            reason,
        }
    }

    fn check_platform(&self) -> Result<WindowsVersion, Refusal> {
        match self.adapter.windows_version() {
            Ok(version) if version.supports_current_release() => Ok(version),
            Ok(_) => Err(Refusal::GateBlocked {
                gate: Gate::Platform,
            }),
            Err(_) => Err(Refusal::GateReadFailed {
                gate: Gate::Platform,
            }),
        }
    }

    fn check_anti_cheat(&self) -> Result<(), Refusal> {
        match self.adapter.anti_cheat_state() {
            Ok(state) if state.active => Err(Refusal::GateBlocked {
                gate: Gate::AntiCheat,
            }),
            Ok(_) => Ok(()),
            Err(_) => Err(Refusal::GateReadFailed {
                gate: Gate::AntiCheat,
            }),
        }
    }

    fn observe_service(&self, service: &str) -> Observation<ServiceState> {
        match self.adapter.service_state(service) {
            Ok(state) => Observation::observed(state),
            Err(failure) => Observation::unavailable(failure),
        }
    }

    fn issued_preview(&self, preview_id: &PreviewId) -> Option<IssuedPreview> {
        lock(&self.previews)
            .iter()
            .find(|issued| &issued.preview.id == preview_id)
            .cloned()
    }

    fn consume_preview(&self, preview_id: &PreviewId) {
        lock(&self.previews).retain(|issued| &issued.preview.id != preview_id);
    }

    fn begin_write(&self) -> WriteStart<'_> {
        let mut gate = lock(&self.write_gate);
        if gate.in_flight {
            return WriteStart::Busy;
        }
        gate.in_flight = true;
        match self.journal.lock_exclusive() {
            Ok(journal) => WriteStart::Acquired(WriteGuard {
                gate: &self.write_gate,
                journal: Some(journal),
            }),
            Err(JournalError::Busy) => {
                gate.in_flight = false;
                WriteStart::Busy
            }
            Err(_) => {
                gate.in_flight = false;
                WriteStart::Unavailable
            }
        }
    }

    fn recovery_required<T>(&self, record: JournalRecord) -> CommandOutcome<T> {
        let status = RecoveryStatus::pending(&record);
        let _ = self.journal.update(record);
        CommandOutcome::RecoveryRequired(status)
    }

    fn next_preview_id(&self) -> PreviewId {
        let sequence = self.preview_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        PreviewId::parse(format!("preview-{sequence}")).expect("generated preview id is valid")
    }

    fn next_transaction_id(&self, records: &[JournalRecord]) -> TransactionId {
        loop {
            let sequence = self.transaction_sequence.fetch_add(1, Ordering::Relaxed) + 1;
            let transaction_id = TransactionId::parse(format!("tx-{sequence}"))
                .expect("generated transaction id is valid");
            if !records
                .iter()
                .any(|record| record.transaction_id == transaction_id)
            {
                return transaction_id;
            }
        }
    }
}

fn recovery_status(records: &[JournalRecord]) -> RecoveryStatus {
    let pending_transactions: Vec<PendingTransaction> = records
        .iter()
        .filter(|record| record.state.is_pending())
        .map(PendingTransaction::from)
        .collect();
    let writes_permitted = pending_transactions.is_empty();
    RecoveryStatus {
        pending_transactions,
        writes_permitted,
        blocker: (!writes_permitted).then_some(WriteBlocker::PendingRecovery),
    }
}

fn platform_for(version: &Observation<WindowsVersion>) -> PlatformSupport {
    match version {
        Observation::Observed { value } if value.supports_current_release() => {
            PlatformSupport::Supported
        }
        Observation::Observed { .. } => PlatformSupport::Unsupported,
        Observation::Unavailable { .. } => PlatformSupport::Unknown,
    }
}

fn disposition_for(
    platform: PlatformSupport,
    observed: &Observation<ServiceState>,
    target: ServiceState,
) -> (Disposition, DispositionReason) {
    match platform {
        PlatformSupport::Unsupported => (
            Disposition::Blocked,
            DispositionReason::UnsupportedWindowsVersion,
        ),
        PlatformSupport::Unknown => (Disposition::Unknown, DispositionReason::PlatformUnknown),
        PlatformSupport::Supported => match observed {
            Observation::Unavailable { .. } => {
                (Disposition::Unknown, DispositionReason::StateUnavailable)
            }
            Observation::Observed { value }
                if value.start_type == ServiceStartType::Unknown
                    || value.run_state == ServiceRunState::Unknown =>
            {
                (Disposition::Unknown, DispositionReason::StateUnavailable)
            }
            Observation::Observed { value } if *value == target => {
                (Disposition::NoOp, DispositionReason::AlreadyAtTarget)
            }
            Observation::Observed { .. } => {
                (Disposition::Applicable, DispositionReason::TargetDiffers)
            }
        },
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
