"""Worker-side assignment flow and message typing."""

from __future__ import annotations

import io
from datetime import UTC, datetime
from uuid import uuid4

import pytest
from pydantic import ValidationError

from harness_ai.assignment import Completed, StartTask, run_mock_assignment
from harness_ai.framing import encode_frame
from harness_ai.protocol import Envelope


def _start(provider: str = "mock") -> Envelope:
    return Envelope(
        protocol_version="1.0",
        kind="start_task",
        message_id=uuid4(),
        correlation_id=None,
        run_id="run-1",
        task_id="task-1",
        attempt=1,
        timestamp=datetime.now(UTC),
        payload={
            "role": "coder",
            "worktree_id": "wt-1",
            "model": {"provider": provider, "model_id": "m"},
            "context_manifest_hash": "sha256:x",
            "allowed_tools": ["repo.read"],
            "max_tool_calls": 5,
            "max_output_tokens": 100,
            "timeout_seconds": 10,
            "cost_cap_usd": 0.0,
        },
    )


def _frames(raw: bytes) -> list[Envelope]:
    return [Envelope.model_validate_json(line) for line in raw.splitlines()]


def test_mock_assignment_completes_with_matching_correlation() -> None:
    out = io.BytesIO()

    class Reply(io.BytesIO):
        """Answers the tool_request the worker has already written to `out`."""

        def readline(self, size: int | None = -1) -> bytes:
            request = _frames(out.getvalue())[1]
            assert request.kind == "tool_request"
            reply = Envelope(
                protocol_version="1.0",
                kind="tool_response",
                message_id=uuid4(),
                correlation_id=str(request.message_id),
                run_id="run-1",
                task_id="task-1",
                attempt=1,
                timestamp=datetime.now(UTC),
                payload={"ok": True, "result": {"content": "x"}},
            )
            return encode_frame(reply)

    run_mock_assignment(_start(), Reply(), out)
    kinds = [f.kind for f in _frames(out.getvalue())]
    assert kinds == ["progress", "tool_request", "usage", "completed"]


def test_unknown_provider_fails() -> None:
    out = io.BytesIO()
    run_mock_assignment(_start("openai"), io.BytesIO(), out)
    (frame,) = _frames(out.getvalue())
    assert frame.kind == "failed"
    assert frame.payload["retryable"] is False


def test_payload_models_are_strict() -> None:
    with pytest.raises(ValidationError):
        Completed.model_validate({"summary": "x", "changed_paths": [], "claimed_tests": [], "z": 1})
    with pytest.raises(ValidationError):
        StartTask.model_validate({"role": "coder"})
