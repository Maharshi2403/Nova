use harness_core::framing::{read_frame, write_frame};
use harness_core::handshake::{HandshakeError, build_hello, verify_ack};
use harness_core::protocol::{Envelope, MessageKind};
use serde_json::json;
use std::io::Cursor;
use uuid::Uuid;

const TS: &str = "2026-11-04T16:00:00Z";

fn ack_for(hello: &Envelope, version: &str) -> Envelope {
    let mut ack = hello.clone();
    ack.kind = MessageKind::HelloAck;
    ack.message_id = Uuid::new_v4();
    ack.correlation_id = Some(hello.message_id.to_string());
    ack.payload.clear();
    ack.payload.insert("runtime".into(), json!("python"));
    ack.payload
        .insert("selected_version".into(), json!(version));
    ack.payload.insert("worker_version".into(), json!("0.1.0"));
    ack
}

#[test]
fn handshake_correlation_matches() {
    let hello = build_hello(Uuid::new_v4(), TS);
    hello.validate().unwrap();
    let accepted = verify_ack(&hello, &ack_for(&hello, "1.0")).unwrap();
    assert_eq!(accepted.worker_version, "0.1.0");

    // An ack for another hello is rejected, as is one with no correlation at all.
    let other = build_hello(Uuid::new_v4(), TS);
    assert!(matches!(
        verify_ack(&hello, &ack_for(&other, "1.0")).unwrap_err(),
        HandshakeError::CorrelationMismatch { .. }
    ));
    let mut uncorrelated = ack_for(&hello, "1.0");
    uncorrelated.correlation_id = None;
    assert!(matches!(
        verify_ack(&hello, &uncorrelated).unwrap_err(),
        HandshakeError::CorrelationMismatch { actual: None, .. }
    ));
}

#[test]
fn rejects_unsupported_version() {
    let hello = build_hello(Uuid::new_v4(), TS);
    for v in ["2.0", "0.9", "1.1", ""] {
        assert!(
            matches!(
                verify_ack(&hello, &ack_for(&hello, v)).unwrap_err(),
                HandshakeError::UnsupportedVersion(_)
            ),
            "{v}"
        );
    }
    // A wire envelope declaring an unknown protocol_version never parses.
    let bad = include_str!("../../../tests/fixtures/protocol/invalid/wrong_version.json");
    assert!(Envelope::from_json(bad).is_err());
    // A version Rust did not offer is rejected even if it is a known one.
    let mut narrow = hello.clone();
    narrow
        .payload
        .insert("supported_versions".into(), json!(["0.9"]));
    assert!(matches!(
        verify_ack(&narrow, &ack_for(&narrow, "1.0")).unwrap_err(),
        HandshakeError::UnsupportedVersion(_)
    ));
}

#[test]
fn rejects_wrong_kinds_and_missing_fields() {
    let hello = build_hello(Uuid::new_v4(), TS);
    assert!(matches!(
        verify_ack(&hello, &hello).unwrap_err(),
        HandshakeError::NotHelloAck(MessageKind::Hello)
    ));
    let ack = ack_for(&hello, "1.0");
    assert!(matches!(
        verify_ack(&ack, &ack).unwrap_err(),
        HandshakeError::NotHello(MessageKind::HelloAck)
    ));
    let mut no_version = ack_for(&hello, "1.0");
    no_version.payload.remove("selected_version");
    assert!(matches!(
        verify_ack(&hello, &no_version).unwrap_err(),
        HandshakeError::MissingField("selected_version")
    ));
}

#[test]
fn handshake_survives_framing() {
    let hello = build_hello(Uuid::new_v4(), TS);
    let mut wire = Vec::new();
    write_frame(&mut wire, &hello).unwrap();
    let parsed = read_frame(&mut Cursor::new(wire)).unwrap().unwrap();
    assert_eq!(parsed, hello);
}
