"""Worker side of the hello/hello_ack handshake."""

from __future__ import annotations

from datetime import UTC, datetime
from uuid import UUID, uuid4

from harness_ai import __version__
from harness_ai.protocol import SUPPORTED_VERSIONS, Envelope


class HandshakeError(Exception):
    """The peer's hello cannot be accepted."""


def build_ack(
    hello: Envelope,
    *,
    message_id: UUID | None = None,
    timestamp: datetime | None = None,
) -> Envelope:
    """Answer a hello, echoing its message_id as correlation_id and picking a version."""
    if hello.kind != "hello":
        raise HandshakeError(f"expected hello, got {hello.kind}")
    offered = hello.payload.get("supported_versions")
    if not isinstance(offered, list) or not all(isinstance(v, str) for v in offered):
        raise HandshakeError("hello.payload.supported_versions must be a list of strings")
    common = [v for v in SUPPORTED_VERSIONS if v in offered]
    if not common:
        raise HandshakeError(
            f"no common protocol version: peer offers {offered}, worker supports "
            f"{list(SUPPORTED_VERSIONS)}"
        )
    return Envelope(
        protocol_version="1.0",
        kind="hello_ack",
        message_id=message_id or uuid4(),
        correlation_id=str(hello.message_id),
        run_id=None,
        task_id=None,
        attempt=None,
        timestamp=timestamp or datetime.now(UTC),
        payload={
            "runtime": "python",
            "selected_version": common[-1],
            "worker_version": __version__,
        },
    )
