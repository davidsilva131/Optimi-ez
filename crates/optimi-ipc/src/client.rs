use std::fmt;

use optimi_domain::command::{
    AbandonedTransaction, CommandOutcome, Consent, RecoveryStatus, Refusal, RestoreReport,
};
use optimi_domain::id::{PreviewId, ProfileId, TransactionId};
use optimi_domain::model::{Preview, VerifiedChange};

use crate::{
    AbandonRequest, ApplyRequest, Command, PROTOCOL_VERSION, PreviewRequest, ProtocolError,
    Request, ResponseBody, RestoreTransactionRequest, UndoRequest, decode_response, encode_request,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    ElevationCancelled,
    Unavailable,
    Disconnected,
    Io,
}

pub trait WorkerTransport: Send {
    fn start(&mut self) -> Result<(), TransportError>;
    fn exchange(&mut self, frame: &[u8]) -> Result<Vec<u8>, TransportError>;
    fn close(&mut self);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerError {
    ElevationCancelled,
    Unavailable,
    Disconnected,
    Protocol(ProtocolError),
    Rejected(ProtocolError),
    PreviewRefused(Refusal),
    UnexpectedReply,
}

impl WorkerError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ElevationCancelled => "elevation_cancelled",
            Self::Unavailable => "unavailable",
            Self::Disconnected => "disconnected",
            Self::Protocol(_) => "protocol",
            Self::Rejected(_) => "rejected",
            Self::PreviewRefused(_) => "preview_refused",
            Self::UnexpectedReply => "unexpected_reply",
        }
    }
}

impl fmt::Display for WorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for WorkerError {}

impl From<TransportError> for WorkerError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::ElevationCancelled => Self::ElevationCancelled,
            TransportError::Unavailable | TransportError::Io => Self::Unavailable,
            TransportError::Disconnected => Self::Disconnected,
        }
    }
}

pub struct WorkerClient<T: WorkerTransport> {
    transport: T,
    started: bool,
    next_request_id: u64,
}

impl<T: WorkerTransport> WorkerClient<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            started: false,
            next_request_id: 1,
        }
    }

    pub fn preview(
        &mut self,
        profile_id: &ProfileId,
    ) -> Result<CommandOutcome<Preview>, WorkerError> {
        match self.send(Command::Preview(PreviewRequest {
            profile_id: profile_id.clone(),
        }))? {
            ResponseBody::Preview(outcome) => Ok(outcome),
            _ => Err(self.unexpected_reply()),
        }
    }

    pub fn apply(
        &mut self,
        preview_id: &PreviewId,
        consent: &Consent,
    ) -> Result<CommandOutcome<VerifiedChange>, WorkerError> {
        match self.send(Command::Apply(ApplyRequest {
            preview_id: preview_id.clone(),
            consent: consent.clone(),
        }))? {
            ResponseBody::Apply(outcome) => Ok(outcome),
            _ => Err(self.unexpected_reply()),
        }
    }

    pub fn apply_reviewed(
        &mut self,
        reviewed: &Preview,
    ) -> Result<CommandOutcome<VerifiedChange>, WorkerError> {
        let fresh = match self.preview(&reviewed.profile_id)? {
            CommandOutcome::Completed(preview) => preview,
            CommandOutcome::Refused(refusal) => return Err(WorkerError::PreviewRefused(refusal)),
            _ => return Err(WorkerError::UnexpectedReply),
        };
        if !matches_review(&fresh, reviewed) {
            return Ok(CommandOutcome::Refused(Refusal::PreviewStateChanged {
                preview_id: reviewed.id.to_string(),
            }));
        }
        let consent = Consent {
            preview_id: fresh.id.clone(),
        };
        self.apply(&fresh.id, &consent)
    }

    pub fn undo(
        &mut self,
        transaction_id: &TransactionId,
    ) -> Result<CommandOutcome<VerifiedChange>, WorkerError> {
        match self.send(Command::Undo(UndoRequest {
            transaction_id: transaction_id.clone(),
        }))? {
            ResponseBody::Undo(outcome) => Ok(outcome),
            _ => Err(self.unexpected_reply()),
        }
    }

    pub fn restore(
        &mut self,
        transaction_id: &TransactionId,
        confirmed_build: Option<u32>,
    ) -> Result<CommandOutcome<VerifiedChange>, WorkerError> {
        match self.send(Command::RestoreTransaction(RestoreTransactionRequest {
            transaction_id: transaction_id.clone(),
            confirmed_build,
        }))? {
            ResponseBody::Restore(outcome) => Ok(outcome),
            _ => Err(self.unexpected_reply()),
        }
    }

    pub fn restore_all(&mut self) -> Result<CommandOutcome<RestoreReport>, WorkerError> {
        match self.send(Command::RestoreAll)? {
            ResponseBody::RestoreAll(outcome) => Ok(outcome),
            _ => Err(self.unexpected_reply()),
        }
    }

    pub fn abandon(
        &mut self,
        transaction_id: &TransactionId,
        service: &str,
    ) -> Result<CommandOutcome<AbandonedTransaction>, WorkerError> {
        match self.send(Command::Abandon(AbandonRequest {
            transaction_id: transaction_id.clone(),
            service: service.to_owned(),
        }))? {
            ResponseBody::Abandon(outcome) => Ok(outcome),
            _ => Err(self.unexpected_reply()),
        }
    }

    pub fn recovery_status(&mut self) -> Result<CommandOutcome<RecoveryStatus>, WorkerError> {
        match self.send(Command::RecoveryStatus)? {
            ResponseBody::RecoveryStatus(outcome) => Ok(outcome),
            _ => Err(self.unexpected_reply()),
        }
    }

    pub fn shutdown(&mut self) {
        if self.started {
            self.transport.close();
            self.started = false;
        }
    }

    fn send(&mut self, command: Command) -> Result<ResponseBody, WorkerError> {
        if !self.started {
            self.transport.start()?;
            self.started = true;
        }
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        let request = Request {
            protocol_version: PROTOCOL_VERSION,
            request_id,
            command,
        };
        let frame = encode_request(&request).map_err(WorkerError::Protocol)?;
        let reply = match self.transport.exchange(&frame) {
            Ok(reply) => reply,
            Err(error) => {
                self.close_session();
                return Err(error.into());
            }
        };
        let response = match decode_response(&reply) {
            Ok(response) => response,
            Err(error) => {
                self.close_session();
                return Err(WorkerError::Protocol(error));
            }
        };
        if response.request_id != request_id {
            self.close_session();
            return Err(WorkerError::UnexpectedReply);
        }
        match response.body {
            ResponseBody::Rejected(error) => Err(WorkerError::Rejected(error)),
            body => Ok(body),
        }
    }

    fn unexpected_reply(&mut self) -> WorkerError {
        self.close_session();
        WorkerError::UnexpectedReply
    }

    fn close_session(&mut self) {
        self.transport.close();
        self.started = false;
    }
}

fn matches_review(fresh: &Preview, reviewed: &Preview) -> bool {
    fresh.profile_id == reviewed.profile_id
        && fresh.catalog_version == reviewed.catalog_version
        && fresh.platform == reviewed.platform
        && fresh.windows_version == reviewed.windows_version
        && fresh.interactive_identity == reviewed.interactive_identity
        && fresh.actions.len() == reviewed.actions.len()
        && fresh
            .actions
            .iter()
            .zip(reviewed.actions.iter())
            .all(|(fresh, reviewed)| {
                fresh.action_id == reviewed.action_id
                    && fresh.service == reviewed.service
                    && fresh.target == reviewed.target
                    && fresh.observed == reviewed.observed
                    && fresh.disposition == reviewed.disposition
            })
}
