use harness_core::messages::*;
use harness_core::protocol::{Envelope, MessageKind, ProtocolVersion};
use serde_json::{Map, Value, json};
use uuid::Uuid;

fn obj(v: Value) -> Map<String, Value> {
    v.as_object().unwrap().clone()
}

fn env(kind: MessageKind, attempt: u32, payload: Value) -> Envelope {
    Envelope {
        protocol_version: ProtocolVersion::V1_0,
        kind,
        message_id: Uuid::new_v4(),
        correlation_id: None,
        run_id: Some("run-1".into()),
        task_id: Some("task-1".into()),
        attempt: Some(attempt),
        timestamp: "2026-01-01T00:00:00Z".into(),
        payload: obj(payload),
    }
}

fn session() -> AssignmentSession {
    AssignmentSession::new("run-1", "task-1", 1, 2)
}

fn tool_req() -> Envelope {
    env(
        MessageKind::ToolRequest,
        1,
        json!({"tool": "repo.read", "arguments": {"path": "a.py"}}),
    )
}

fn ok_response() -> ToolResponse {
    ToolResponse {
        ok: true,
        result: Some(json!({"content": "x"})),
        error: None,
    }
}

#[test]
fn tool_response_matches_request() {
    let mut s = session();
    let req = tool_req();
    let WorkerMessage::ToolRequest { request_id, .. } = s.accept(&req).unwrap() else {
        panic!("expected tool request");
    };
    assert_eq!(request_id, req.message_id);
    // Unknown id is refused and leaves the request outstanding.
    assert!(matches!(
        s.tool_response(Uuid::new_v4(), &ok_response(), "t".into()),
        Err(SessionError::UnknownCorrelation(_))
    ));
    let resp = s
        .tool_response(request_id, &ok_response(), "t".into())
        .unwrap();
    assert_eq!(
        resp.correlation_id.as_deref(),
        Some(request_id.to_string().as_str())
    );
    assert_eq!(resp.kind, MessageKind::ToolResponse);
    resp.validate().unwrap();
    // Cannot answer twice.
    assert_eq!(
        s.tool_response(request_id, &ok_response(), "t".into()),
        Err(SessionError::NoToolRequestOutstanding)
    );
}

#[test]
fn one_outstanding_request_and_call_limit() {
    let mut s = session();
    s.accept(&tool_req()).unwrap();
    assert_eq!(
        s.accept(&tool_req()),
        Err(SessionError::ToolRequestOutstanding)
    );
    let id = s.pending_request().unwrap();
    s.tool_response(id, &ok_response(), "t".into()).unwrap();
    s.accept(&tool_req()).unwrap();
    let id = s.pending_request().unwrap();
    s.tool_response(id, &ok_response(), "t".into()).unwrap();
    assert_eq!(s.accept(&tool_req()), Err(SessionError::ToolCallLimit(2)));
}

#[test]
fn invalid_completion_rejected() {
    let mut s = session();
    let good = json!({"summary": "done", "changed_paths": [], "claimed_tests": []});
    // Empty summary, missing fields, unknown fields, wrong attempt.
    for bad in [
        env(
            MessageKind::Completed,
            1,
            json!({"summary": " ", "changed_paths": [], "claimed_tests": []}),
        ),
        env(MessageKind::Completed, 1, json!({"summary": "x"})),
        env(
            MessageKind::Completed,
            1,
            json!({"summary": "x", "changed_paths": [], "claimed_tests": [], "extra": 1}),
        ),
    ] {
        assert!(matches!(s.accept(&bad), Err(SessionError::BadPayload(..))));
    }
    assert_eq!(
        s.accept(&env(MessageKind::Completed, 2, good.clone())),
        Err(SessionError::WrongScope)
    );
    // Cannot complete with a tool request still open.
    s.accept(&tool_req()).unwrap();
    assert_eq!(
        s.accept(&env(MessageKind::Completed, 1, good.clone())),
        Err(SessionError::CompletedWithOpenRequest)
    );
    let id = s.pending_request().unwrap();
    s.tool_response(id, &ok_response(), "t".into()).unwrap();
    assert!(matches!(
        s.accept(&env(MessageKind::Completed, 1, good.clone())),
        Ok(WorkerMessage::Completed(_))
    ));
    assert!(s.is_finished());
    assert_eq!(
        s.accept(&env(
            MessageKind::Failed,
            1,
            json!({"reason": "r", "retryable": false})
        )),
        Err(SessionError::AfterTerminal)
    );
}

#[test]
fn duplicate_message_ids_rejected() {
    let mut s = session();
    let p = env(MessageKind::Progress, 1, json!({"message": "hi"}));
    s.accept(&p).unwrap();
    assert_eq!(s.accept(&p), Err(SessionError::Duplicate(p.message_id)));
}

#[test]
fn usage_keeps_unknown_as_null() {
    let mut s = session();
    let u = env(
        MessageKind::Usage,
        1,
        json!({"provider": "mock", "model_id": "m", "input_tokens": null,
               "output_tokens": null, "cost_usd": null, "price_source": "unknown"}),
    );
    let Ok(WorkerMessage::Usage(usage)) = s.accept(&u) else {
        panic!("usage rejected");
    };
    assert_eq!(usage.input_tokens, None);
}

#[test]
fn failed_message_and_bad_tool_response_shapes() {
    let mut s = session();
    s.accept(&tool_req()).unwrap();
    let id = s.pending_request().unwrap();
    let bad = ToolResponse {
        ok: true,
        result: None,
        error: None,
    };
    assert!(matches!(
        s.tool_response(id, &bad, "t".into()),
        Err(SessionError::BadPayload(..))
    ));
    let err = ToolResponse {
        ok: false,
        result: None,
        error: Some("denied".into()),
    };
    s.tool_response(id, &err, "t".into()).unwrap();
    assert!(matches!(
        s.accept(&env(
            MessageKind::Failed,
            1,
            json!({"reason": "denied", "retryable": true})
        )),
        Ok(WorkerMessage::Failed(_))
    ));
}
