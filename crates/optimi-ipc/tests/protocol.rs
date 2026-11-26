mod support;

use std::io::Cursor;

use optimi_domain::command::{CommandOutcome, Consent};
use optimi_domain::id::{PreviewId, ProfileId, TransactionId};
use optimi_ipc::{
    Command, FrameReadError, MAX_FRAME_BYTES, PROTOCOL_VERSION, ProtocolError, Request, Response,
    ResponseBody, decode_request, decode_response, encode_request, encode_response, read_frame,
};

fn request(request_id: u64, command: Command) -> Request {
    Request {
        protocol_version: PROTOCOL_VERSION,
        request_id,
        command,
    }
}

fn every_command() -> Vec<Command> {
    let preview_id = PreviewId::parse("preview-1").expect("preview id is valid");
    let transaction_id = TransactionId::parse("tx-1").expect("transaction id is valid");
    vec![
        Command::Preview(optimi_ipc::PreviewRequest {
            profile_id: ProfileId::parse("competitive").expect("profile id is valid"),
        }),
        Command::Apply(optimi_ipc::ApplyRequest {
            preview_id: preview_id.clone(),
            consent: Consent { preview_id },
        }),
        Command::Undo(optimi_ipc::UndoRequest {
            transaction_id: transaction_id.clone(),
        }),
        Command::RestoreTransaction(optimi_ipc::RestoreTransactionRequest {
            transaction_id: transaction_id.clone(),
            confirmed_build: Some(22631),
        }),
        Command::RestoreAll,
        Command::Abandon(optimi_ipc::AbandonRequest {
            transaction_id,
            service: "DiagTrack".to_owned(),
        }),
        Command::RecoveryStatus,
    ]
}

fn frame_with_declared_length(declared: usize) -> Vec<u8> {
    let mut frame = Vec::with_capacity(4);
    frame.extend_from_slice(&(declared as u32).to_le_bytes());
    frame
}

#[test]
fn every_domain_command_round_trips_through_the_wire_format() {
    for (index, command) in every_command().into_iter().enumerate() {
        let request = request(index as u64 + 1, command.clone());
        let frame = encode_request(&request).expect("request encodes");
        let decoded = decode_request(&frame).expect("request decodes");

        assert_eq!(decoded, request);
    }
}

#[test]
fn an_unknown_command_name_is_rejected() {
    let frame = support::raw_frame(
        r#"{"protocol_version":1,"request_id":1,"command":{"name":"shell","params":{"command":"whoami"}}}"#,
    );

    assert_eq!(decode_request(&frame), Err(ProtocolError::Malformed));
}

#[test]
fn an_unknown_field_inside_a_command_is_rejected() {
    let frame = support::raw_frame(
        r#"{"protocol_version":1,"request_id":1,"command":{"name":"preview","params":{"profile_id":"competitive","service":"DiagTrack"}}}"#,
    );

    assert_eq!(decode_request(&frame), Err(ProtocolError::Malformed));
}

#[test]
fn an_unknown_field_inside_the_envelope_is_rejected() {
    let frame = support::raw_frame(
        r#"{"protocol_version":1,"request_id":1,"command":{"name":"restore_all"},"elevate":true}"#,
    );

    assert_eq!(decode_request(&frame), Err(ProtocolError::Malformed));
}

#[test]
fn a_command_from_another_protocol_version_is_rejected() {
    let frame = support::raw_frame(
        r#"{"protocol_version":2,"request_id":1,"command":{"name":"recovery_status"}}"#,
    );

    assert_eq!(
        decode_request(&frame),
        Err(ProtocolError::UnsupportedVersion {
            found: 2,
            supported: PROTOCOL_VERSION
        })
    );
}

#[test]
fn a_message_whose_declared_size_exceeds_the_limit_is_rejected() {
    let frame = frame_with_declared_length(MAX_FRAME_BYTES + 1);

    assert_eq!(
        decode_request(&frame),
        Err(ProtocolError::FrameTooLarge {
            limit: MAX_FRAME_BYTES
        })
    );
}

#[test]
fn an_oversized_stream_frame_is_rejected_before_reading_the_body() {
    let mut frame = frame_with_declared_length(MAX_FRAME_BYTES + 1);
    frame.extend_from_slice(b"{\"protocol_version\":1");
    let mut reader = Cursor::new(frame);

    assert_eq!(read_frame(&mut reader), Err(FrameReadError::TooLarge));
    assert_eq!(reader.position(), 4);
}

#[test]
fn an_empty_frame_is_rejected() {
    assert_eq!(
        decode_request(&frame_with_declared_length(0)),
        Err(ProtocolError::EmptyFrame)
    );
    assert_eq!(
        read_frame(&mut Cursor::new(frame_with_declared_length(0))),
        Err(FrameReadError::Empty)
    );
}

#[test]
fn malformed_json_and_trailing_bytes_are_rejected() {
    let malformed = support::raw_frame("{\"protocol_version\":1,");
    assert_eq!(decode_request(&malformed), Err(ProtocolError::Malformed));

    let mut trailing = support::raw_frame(
        r#"{"protocol_version":1,"request_id":1,"command":{"name":"recovery_status"}}"#,
    );
    trailing.extend_from_slice(b"x");
    assert_eq!(decode_request(&trailing), Err(ProtocolError::Malformed));
}

#[test]
fn a_truncated_length_prefix_is_rejected() {
    assert_eq!(decode_request(b"{}"), Err(ProtocolError::Malformed));
}

#[test]
fn an_invalid_catalog_identifier_is_rejected_by_the_schema() {
    let frame = support::raw_frame(
        r#"{"protocol_version":1,"request_id":1,"command":{"name":"preview","params":{"profile_id":"../admin"}}}"#,
    );

    assert_eq!(decode_request(&frame), Err(ProtocolError::Malformed));
}

#[test]
fn a_frame_without_a_body_is_reported_as_io_not_as_a_command() {
    let mut frame = frame_with_declared_length(10);
    frame.extend_from_slice(b"abc");

    assert_eq!(read_frame(&mut Cursor::new(frame)), Err(FrameReadError::Io));
}

#[test]
fn responses_round_trip_and_check_the_protocol_version() {
    let response = Response {
        protocol_version: PROTOCOL_VERSION,
        request_id: 7,
        body: ResponseBody::Preview(CommandOutcome::Completed(support::preview("preview-1"))),
    };
    let frame = encode_response(&response).expect("response encodes");

    assert_eq!(decode_response(&frame), Ok(response));

    let foreign = Response {
        protocol_version: PROTOCOL_VERSION + 1,
        request_id: 7,
        body: ResponseBody::Rejected(ProtocolError::Malformed),
    };
    let frame = encode_response(&foreign).expect("response encodes");
    assert_eq!(
        decode_response(&frame),
        Err(ProtocolError::UnsupportedVersion {
            found: PROTOCOL_VERSION + 1,
            supported: PROTOCOL_VERSION
        })
    );
}

#[test]
fn an_oversized_response_cannot_be_encoded() {
    let mut preview = support::preview("preview-1");
    preview.actions[0].evidence.source = "x".repeat(MAX_FRAME_BYTES);
    let response = Response {
        protocol_version: PROTOCOL_VERSION,
        request_id: 1,
        body: ResponseBody::Preview(CommandOutcome::Completed(preview)),
    };

    assert_eq!(
        encode_response(&response),
        Err(ProtocolError::FrameTooLarge {
            limit: MAX_FRAME_BYTES
        })
    );
}

#[test]
fn pipe_names_are_restricted_to_the_worker_prefix_and_a_hex_nonce() {
    assert!(optimi_ipc::is_valid_pipe_name(
        "optimi-ez-worker-00112233445566778899aabbccddeeff"
    ));
    assert!(!optimi_ipc::is_valid_pipe_name("optimi-ez-worker-"));
    assert!(!optimi_ipc::is_valid_pipe_name(
        "optimi-ez-worker-zz112233445566778899aabbccddeeff"
    ));
    assert!(!optimi_ipc::is_valid_pipe_name("other-0011223344556677"));
    assert_eq!(
        optimi_ipc::pipe_name(&[0xab; 16]),
        "optimi-ez-worker-abababababababababababababababab"
    );
}
