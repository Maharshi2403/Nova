from pathlib import Path

import pytest

from harness_ai import __version__
from harness_ai.handshake import HandshakeError, build_ack
from harness_ai.protocol import Envelope

FIXTURES = Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "protocol"


def load(name: str) -> Envelope:
    return Envelope.model_validate_json((FIXTURES / "valid" / name).read_text(encoding="utf-8"))


def test_handshake_correlation_matches() -> None:
    hello = load("hello.json")
    ack = build_ack(hello)
    assert ack.kind == "hello_ack"
    assert ack.correlation_id == str(hello.message_id)
    assert ack.message_id != hello.message_id
    assert ack.payload["selected_version"] == "1.0"
    assert ack.payload["worker_version"] == __version__
    assert ack.run_id is None and ack.task_id is None and ack.attempt is None


def test_rejects_unsupported_version() -> None:
    hello = load("hello.json").model_copy(
        update={"payload": {"runtime": "rust", "supported_versions": ["2.0"]}}
    )
    with pytest.raises(HandshakeError, match="no common protocol version"):
        build_ack(hello)
    for bad in ({"runtime": "rust"}, {"supported_versions": "1.0"}, {"supported_versions": [1]}):
        with pytest.raises(HandshakeError):
            build_ack(hello.model_copy(update={"payload": bad}))


def test_rejects_non_hello() -> None:
    with pytest.raises(HandshakeError, match="expected hello"):
        build_ack(load("start_task.json"))
