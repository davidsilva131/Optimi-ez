use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::adapter::SystemAdapter;
use crate::catalog::{Catalog, CatalogAction};
use crate::id::{PreviewId, ProfileId, TransactionId};
use crate::model::{
    ActionPreview, Disposition, DispositionReason, Observation, PlatformSupport, Preview,
    ServiceStartType, ServiceState, WindowsVersion,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Consent {
    pub preview_id: PreviewId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteCommand {
    Apply,
    Undo,
    RestoreAll,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Refusal {
    UnknownProfile { profile_id: String },
    InvalidPreviewId { value: String },
    InvalidTransactionId { value: String },
    ConsentMismatch,
    WriteNotImplemented { command: WriteCommand },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteBlocker {
    WriteSupportNotImplemented,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecoveryStatus {
    pub pending_transactions: Vec<TransactionId>,
    pub writes_permitted: bool,
    pub blocker: Option<WriteBlocker>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", content = "payload", rename_all = "snake_case")]
pub enum CommandOutcome<T> {
    Completed(T),
    Refused(Refusal),
    RecoveryRequired(RecoveryStatus),
}

pub struct DomainService<A> {
    adapter: A,
    catalog: Catalog,
    preview_sequence: AtomicU64,
}

impl<A: SystemAdapter> DomainService<A> {
    pub fn new(adapter: A) -> Self {
        Self {
            adapter,
            catalog: Catalog::embedded(),
            preview_sequence: AtomicU64::new(0),
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
        CommandOutcome::Completed(Preview {
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
        })
    }

    pub fn apply(
        &self,
        preview_id: &PreviewId,
        consent: &Consent,
    ) -> CommandOutcome<TransactionId> {
        if &consent.preview_id != preview_id {
            return CommandOutcome::Refused(Refusal::ConsentMismatch);
        }
        CommandOutcome::Refused(Refusal::WriteNotImplemented {
            command: WriteCommand::Apply,
        })
    }

    pub fn undo(&self, _transaction_id: &TransactionId) -> CommandOutcome<()> {
        CommandOutcome::Refused(Refusal::WriteNotImplemented {
            command: WriteCommand::Undo,
        })
    }

    pub fn restore_all(&self) -> CommandOutcome<Vec<TransactionId>> {
        CommandOutcome::Refused(Refusal::WriteNotImplemented {
            command: WriteCommand::RestoreAll,
        })
    }

    pub fn recovery_status(&self) -> CommandOutcome<RecoveryStatus> {
        CommandOutcome::Completed(RecoveryStatus {
            pending_transactions: Vec::new(),
            writes_permitted: false,
            blocker: Some(WriteBlocker::WriteSupportNotImplemented),
        })
    }

    fn action_preview(&self, action: &CatalogAction, platform: PlatformSupport) -> ActionPreview {
        let observed = match self.adapter.service_state(&action.service) {
            Ok(state) => Observation::observed(state),
            Err(failure) => Observation::unavailable(failure),
        };
        let (disposition, reason) = disposition_for(platform, &observed, action.target_start_type);
        ActionPreview {
            action_id: action.id.clone(),
            service: action.service.clone(),
            risk: action.risk,
            evidence: action.evidence.clone(),
            target_start_type: action.target_start_type,
            observed,
            disposition,
            reason,
        }
    }

    fn next_preview_id(&self) -> PreviewId {
        let sequence = self.preview_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        PreviewId::parse(format!("preview-{sequence}")).expect("generated preview id is valid")
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
    target: ServiceStartType,
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
            Observation::Observed { value } if value.start_type == ServiceStartType::Unknown => {
                (Disposition::Unknown, DispositionReason::StateUnavailable)
            }
            Observation::Observed { value } if value.start_type == target => {
                (Disposition::NoOp, DispositionReason::AlreadyAtTarget)
            }
            Observation::Observed { .. } => {
                (Disposition::Applicable, DispositionReason::TargetDiffers)
            }
        },
    }
}
