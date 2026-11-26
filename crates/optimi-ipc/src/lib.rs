use std::io::{Read, Write};

use optimi_domain::command::{
    AbandonedTransaction, CommandOutcome, Consent, RecoveryStatus, RestoreReport,
};
use optimi_domain::id::{PreviewId, ProfileId, TransactionId};
use optimi_domain::model::{Preview, VerifiedChange};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub mod client;
#[cfg(windows)]
pub mod identity;
#[cfg(windows)]
pub mod pipe;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const FRAME_LENGTH_BYTES: usize = 4;
pub const PIPE_NAME_PREFIX: &str = "optimi-ez-worker-";
pub const PIPE_NAME_NONCE_BYTES: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub protocol_version: u16,
    pub request_id: u64,
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "name", content = "params", rename_all = "snake_case")]
pub enum Command {
    Preview(PreviewRequest),
    Apply(ApplyRequest),
    Undo(UndoRequest),
    RestoreTransaction(RestoreTransactionRequest),
    RestoreAll,
    Abandon(AbandonRequest),
    RecoveryStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewRequest {
    pub profile_id: ProfileId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyRequest {
    pub preview_id: PreviewId,
    pub consent: Consent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoRequest {
    pub transaction_id: TransactionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreTransactionRequest {
    pub transaction_id: TransactionId,
    pub confirmed_build: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbandonRequest {
    pub transaction_id: TransactionId,
    pub service: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub protocol_version: u16,
    pub request_id: u64,
    pub body: ResponseBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ResponseBody {
    Preview(CommandOutcome<Preview>),
    Apply(CommandOutcome<VerifiedChange>),
    Undo(CommandOutcome<VerifiedChange>),
    Restore(CommandOutcome<VerifiedChange>),
    RestoreAll(CommandOutcome<RestoreReport>),
    Abandon(CommandOutcome<AbandonedTransaction>),
    RecoveryStatus(CommandOutcome<RecoveryStatus>),
    Rejected(ProtocolError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum ProtocolError {
    EmptyFrame,
    FrameTooLarge { limit: usize },
    Malformed,
    UnsupportedVersion { found: u16, supported: u16 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameReadError {
    Io,
    Empty,
    TooLarge,
}

pub fn is_valid_pipe_name(name: &str) -> bool {
    name.strip_prefix(PIPE_NAME_PREFIX).is_some_and(|nonce| {
        nonce.len() == PIPE_NAME_NONCE_BYTES * 2 && nonce.chars().all(|c| c.is_ascii_hexdigit())
    })
}

pub fn pipe_name(nonce: &[u8]) -> String {
    let mut name = String::from(PIPE_NAME_PREFIX);
    for byte in nonce {
        name.push_str(&format!("{byte:02x}"));
    }
    name
}

pub fn encode_request(request: &Request) -> Result<Vec<u8>, ProtocolError> {
    encode_frame(request)
}

pub fn decode_request(frame: &[u8]) -> Result<Request, ProtocolError> {
    let request: Request = decode_frame(frame)?;
    if request.protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion {
            found: request.protocol_version,
            supported: PROTOCOL_VERSION,
        });
    }
    Ok(request)
}

pub fn encode_response(response: &Response) -> Result<Vec<u8>, ProtocolError> {
    encode_frame(response)
}

pub fn decode_response(frame: &[u8]) -> Result<Response, ProtocolError> {
    let response: Response = decode_frame(frame)?;
    if response.protocol_version != PROTOCOL_VERSION {
        return Err(ProtocolError::UnsupportedVersion {
            found: response.protocol_version,
            supported: PROTOCOL_VERSION,
        });
    }
    Ok(response)
}

pub fn write_frame<W: Write>(writer: &mut W, frame: &[u8]) -> std::io::Result<()> {
    writer.write_all(frame)?;
    writer.flush()
}

pub fn read_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>, FrameReadError> {
    let mut header = [0u8; FRAME_LENGTH_BYTES];
    reader
        .read_exact(&mut header)
        .map_err(|_| FrameReadError::Io)?;
    let declared = u32::from_le_bytes(header) as usize;
    if declared == 0 {
        return Err(FrameReadError::Empty);
    }
    if declared > MAX_FRAME_BYTES {
        return Err(FrameReadError::TooLarge);
    }
    let mut frame = vec![0u8; FRAME_LENGTH_BYTES + declared];
    frame[..FRAME_LENGTH_BYTES].copy_from_slice(&header);
    reader
        .read_exact(&mut frame[FRAME_LENGTH_BYTES..])
        .map_err(|_| FrameReadError::Io)?;
    Ok(frame)
}

fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    let payload = serde_json::to_vec(value).map_err(|_| ProtocolError::Malformed)?;
    if payload.is_empty() {
        return Err(ProtocolError::EmptyFrame);
    }
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            limit: MAX_FRAME_BYTES,
        });
    }
    let mut frame = Vec::with_capacity(FRAME_LENGTH_BYTES + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode_frame<T: DeserializeOwned>(frame: &[u8]) -> Result<T, ProtocolError> {
    if frame.len() < FRAME_LENGTH_BYTES {
        return Err(ProtocolError::Malformed);
    }
    let declared = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if declared == 0 {
        return Err(ProtocolError::EmptyFrame);
    }
    if declared > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge {
            limit: MAX_FRAME_BYTES,
        });
    }
    if frame.len() != FRAME_LENGTH_BYTES + declared {
        return Err(ProtocolError::Malformed);
    }
    serde_json::from_slice(&frame[FRAME_LENGTH_BYTES..]).map_err(|_| ProtocolError::Malformed)
}
