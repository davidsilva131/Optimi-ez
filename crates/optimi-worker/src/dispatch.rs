use optimi_domain::adapter::SystemAdapter;
use optimi_domain::command::{
    AbandonConsent, AbandonedTransaction, BuildChangeConfirmation, CommandOutcome, Consent,
    DomainService, RecoveryStatus, RestoreReport,
};
use optimi_domain::id::{PreviewId, ProfileId, TransactionId};
use optimi_domain::journal::JournalStore;
use optimi_domain::model::{Preview, VerifiedChange};
use optimi_ipc::{
    Command, PROTOCOL_VERSION, Request, Response, ResponseBody, decode_request, encode_response,
};

pub trait WorkerCommands {
    fn preview(&self, profile_id: &ProfileId) -> CommandOutcome<Preview>;
    fn apply(&self, preview_id: &PreviewId, consent: &Consent) -> CommandOutcome<VerifiedChange>;
    fn undo(&self, transaction_id: &TransactionId) -> CommandOutcome<VerifiedChange>;
    fn restore_transaction(
        &self,
        transaction_id: &TransactionId,
        confirmed_build: Option<u32>,
    ) -> CommandOutcome<VerifiedChange>;
    fn restore_all(&self) -> CommandOutcome<RestoreReport>;
    fn abandon(
        &self,
        transaction_id: &TransactionId,
        service: &str,
    ) -> CommandOutcome<AbandonedTransaction>;
    fn recovery_status(&self) -> CommandOutcome<RecoveryStatus>;
}

impl<A: SystemAdapter, J: JournalStore> WorkerCommands for DomainService<A, J> {
    fn preview(&self, profile_id: &ProfileId) -> CommandOutcome<Preview> {
        DomainService::preview(self, profile_id)
    }

    fn apply(&self, preview_id: &PreviewId, consent: &Consent) -> CommandOutcome<VerifiedChange> {
        DomainService::apply(self, preview_id, consent)
    }

    fn undo(&self, transaction_id: &TransactionId) -> CommandOutcome<VerifiedChange> {
        DomainService::undo(self, transaction_id)
    }

    fn restore_transaction(
        &self,
        transaction_id: &TransactionId,
        confirmed_build: Option<u32>,
    ) -> CommandOutcome<VerifiedChange> {
        match confirmed_build {
            Some(confirmed_build) => self.restore_after_build_change(&BuildChangeConfirmation {
                transaction_id: transaction_id.clone(),
                confirmed_build,
            }),
            None => DomainService::restore_transaction(self, transaction_id),
        }
    }

    fn restore_all(&self) -> CommandOutcome<RestoreReport> {
        DomainService::restore_all(self)
    }

    fn abandon(
        &self,
        transaction_id: &TransactionId,
        service: &str,
    ) -> CommandOutcome<AbandonedTransaction> {
        self.abandon(&AbandonConsent {
            transaction_id: transaction_id.clone(),
            service: service.to_owned(),
        })
    }

    fn recovery_status(&self) -> CommandOutcome<RecoveryStatus> {
        DomainService::recovery_status(self)
    }
}

pub fn dispatch<C: WorkerCommands>(commands: &C, frame: &[u8]) -> Vec<u8> {
    let response = match decode_request(frame) {
        Ok(request) => Response {
            protocol_version: PROTOCOL_VERSION,
            request_id: request.request_id,
            body: handle(commands, request),
        },
        Err(error) => Response {
            protocol_version: PROTOCOL_VERSION,
            request_id: 0,
            body: ResponseBody::Rejected(error),
        },
    };
    encode_response(&response).unwrap_or_default()
}

pub fn rejection(error: optimi_ipc::ProtocolError) -> Vec<u8> {
    let response = Response {
        protocol_version: PROTOCOL_VERSION,
        request_id: 0,
        body: ResponseBody::Rejected(error),
    };
    encode_response(&response).unwrap_or_default()
}

fn handle<C: WorkerCommands>(commands: &C, request: Request) -> ResponseBody {
    match request.command {
        Command::Preview(request) => ResponseBody::Preview(commands.preview(&request.profile_id)),
        Command::Apply(request) => {
            ResponseBody::Apply(commands.apply(&request.preview_id, &request.consent))
        }
        Command::Undo(request) => ResponseBody::Undo(commands.undo(&request.transaction_id)),
        Command::RestoreTransaction(request) => ResponseBody::Restore(
            commands.restore_transaction(&request.transaction_id, request.confirmed_build),
        ),
        Command::RestoreAll => ResponseBody::RestoreAll(commands.restore_all()),
        Command::Abandon(request) => {
            ResponseBody::Abandon(commands.abandon(&request.transaction_id, &request.service))
        }
        Command::RecoveryStatus => ResponseBody::RecoveryStatus(commands.recovery_status()),
    }
}
