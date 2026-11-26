use optimi_domain::adapter::SystemAdapter;
use optimi_domain::command::{
    AbandonedTransaction, CommandFailure, CommandOutcome, Consent, DomainService, RecoveryStatus,
    Refusal, RestoreReport,
};
use optimi_domain::id::{PreviewId, ProfileId, TransactionId};
use optimi_domain::journal::JournalStore;
use optimi_domain::model::{Preview, VerifiedChange};
use optimi_ipc::client::{WorkerClient, WorkerError, WorkerTransport};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", content = "payload", rename_all = "snake_case")]
pub enum UiOutcome<T> {
    Completed(T),
    Refused(Refusal),
    RecoveryRequired(RecoveryStatus),
    Failed(CommandFailure),
    ElevationCancelled,
    WorkerUnavailable { detail: &'static str },
}

impl<T> From<CommandOutcome<T>> for UiOutcome<T> {
    fn from(outcome: CommandOutcome<T>) -> Self {
        match outcome {
            CommandOutcome::Completed(value) => Self::Completed(value),
            CommandOutcome::Refused(refusal) => Self::Refused(refusal),
            CommandOutcome::RecoveryRequired(status) => Self::RecoveryRequired(status),
            CommandOutcome::Failed(failure) => Self::Failed(failure),
        }
    }
}

pub trait WorkerGateway: Send {
    fn apply(&mut self, reviewed: &Preview) -> Result<CommandOutcome<VerifiedChange>, WorkerError>;
    fn undo(
        &mut self,
        transaction_id: &TransactionId,
    ) -> Result<CommandOutcome<VerifiedChange>, WorkerError>;
    fn restore(
        &mut self,
        transaction_id: &TransactionId,
        confirmed_build: Option<u32>,
    ) -> Result<CommandOutcome<VerifiedChange>, WorkerError>;
    fn restore_all(&mut self) -> Result<CommandOutcome<RestoreReport>, WorkerError>;
    fn abandon(
        &mut self,
        transaction_id: &TransactionId,
        service: &str,
    ) -> Result<CommandOutcome<AbandonedTransaction>, WorkerError>;
    fn shutdown(&mut self);
}

impl<T: WorkerTransport> WorkerGateway for WorkerClient<T> {
    fn apply(&mut self, reviewed: &Preview) -> Result<CommandOutcome<VerifiedChange>, WorkerError> {
        self.apply_reviewed(reviewed)
    }

    fn undo(
        &mut self,
        transaction_id: &TransactionId,
    ) -> Result<CommandOutcome<VerifiedChange>, WorkerError> {
        WorkerClient::undo(self, transaction_id)
    }

    fn restore(
        &mut self,
        transaction_id: &TransactionId,
        confirmed_build: Option<u32>,
    ) -> Result<CommandOutcome<VerifiedChange>, WorkerError> {
        WorkerClient::restore(self, transaction_id, confirmed_build)
    }

    fn restore_all(&mut self) -> Result<CommandOutcome<RestoreReport>, WorkerError> {
        WorkerClient::restore_all(self)
    }

    fn abandon(
        &mut self,
        transaction_id: &TransactionId,
        service: &str,
    ) -> Result<CommandOutcome<AbandonedTransaction>, WorkerError> {
        WorkerClient::abandon(self, transaction_id, service)
    }

    fn shutdown(&mut self) {
        WorkerClient::shutdown(self);
    }
}

pub struct Session<A: SystemAdapter, J: JournalStore, W: WorkerGateway> {
    reads: DomainService<A, J>,
    worker: W,
    last_preview: Option<Preview>,
}

impl<A: SystemAdapter, J: JournalStore, W: WorkerGateway> Session<A, J, W> {
    pub fn new(adapter: A, journal: J, worker: W) -> Self {
        Self {
            reads: DomainService::new(adapter, journal),
            worker,
            last_preview: None,
        }
    }

    pub fn preview(&mut self, profile_id: &ProfileId) -> CommandOutcome<Preview> {
        let outcome = self.reads.preview(profile_id);
        if let CommandOutcome::Completed(preview) = &outcome {
            self.last_preview = Some(preview.clone());
        }
        outcome
    }

    pub fn recovery_status(&self) -> CommandOutcome<RecoveryStatus> {
        self.reads.recovery_status()
    }

    pub fn apply(
        &mut self,
        preview_id: &PreviewId,
        consent: &Consent,
    ) -> UiOutcome<VerifiedChange> {
        if &consent.preview_id != preview_id {
            return UiOutcome::Refused(Refusal::ConsentMismatch);
        }
        let reviewed = self
            .last_preview
            .as_ref()
            .filter(|preview| &preview.id == preview_id)
            .cloned();
        let Some(reviewed) = reviewed else {
            return UiOutcome::Refused(Refusal::PreviewNotCurrent {
                preview_id: preview_id.to_string(),
            });
        };
        worker_outcome(self.worker.apply(&reviewed))
    }

    pub fn undo(&mut self, transaction_id: &TransactionId) -> UiOutcome<VerifiedChange> {
        worker_outcome(self.worker.undo(transaction_id))
    }

    pub fn restore(
        &mut self,
        transaction_id: &TransactionId,
        confirmed_build: Option<u32>,
    ) -> UiOutcome<VerifiedChange> {
        worker_outcome(self.worker.restore(transaction_id, confirmed_build))
    }

    pub fn restore_all(&mut self) -> UiOutcome<RestoreReport> {
        worker_outcome(self.worker.restore_all())
    }

    pub fn abandon(
        &mut self,
        transaction_id: &TransactionId,
        service: &str,
    ) -> UiOutcome<AbandonedTransaction> {
        worker_outcome(self.worker.abandon(transaction_id, service))
    }

    pub fn shutdown(&mut self) {
        self.worker.shutdown();
    }
}

fn worker_outcome<T>(result: Result<CommandOutcome<T>, WorkerError>) -> UiOutcome<T> {
    match result {
        Ok(outcome) => outcome.into(),
        Err(WorkerError::ElevationCancelled) => UiOutcome::ElevationCancelled,
        Err(WorkerError::PreviewRefused(refusal)) => UiOutcome::Refused(refusal),
        Err(error) => UiOutcome::WorkerUnavailable {
            detail: error.code(),
        },
    }
}
