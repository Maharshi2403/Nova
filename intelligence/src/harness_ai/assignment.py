"""Typed task-assignment payloads (mirrors harness-core::messages) and the mock assignment."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any, BinaryIO, Literal
from uuid import UUID, uuid4

from pydantic import BaseModel, ConfigDict

from harness_ai.framing import read_frame, write_frame
from harness_ai.protocol import Envelope, MessageKind


class _Strict(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)


class ModelRef(_Strict):
    provider: str
    model_id: str


class StartTask(_Strict):
    role: str
    worktree_id: str
    model: ModelRef
    context_manifest_hash: str
    allowed_tools: list[str]
    max_tool_calls: int
    max_output_tokens: int
    timeout_seconds: int
    cost_cap_usd: float


class ToolRequest(_Strict):
    tool: str
    arguments: dict[str, Any]


class ToolResponse(_Strict):
    ok: bool
    result: Any | None = None
    error: str | None = None


class Usage(_Strict):
    provider: str
    model_id: str
    input_tokens: int | None
    output_tokens: int | None
    cost_usd: float | None
    price_source: Literal["registry_estimate", "provider_billed", "unknown"]


class Completed(_Strict):
    summary: str
    changed_paths: list[str]
    claimed_tests: list[str]
    subtask_requests: list[dict[str, Any]] = []


class Failed(_Strict):
    reason: str
    retryable: bool


class AssignmentError(Exception):
    """The peer broke the assignment protocol."""


def _envelope(
    start: Envelope, kind: MessageKind, payload: dict[str, Any], correlation: UUID | None = None
) -> Envelope:
    return Envelope(
        protocol_version="1.0",
        kind=kind,
        message_id=uuid4(),
        correlation_id=str(correlation) if correlation else None,
        run_id=start.run_id,
        task_id=start.task_id,
        attempt=start.attempt,
        timestamp=datetime.now(UTC),
        payload=payload,
    )


def run_mock_assignment(start: Envelope, stdin: BinaryIO, stdout: BinaryIO) -> None:
    """Deterministic worker behaviour used to exercise the protocol end to end.

    It reads one file through the broker, reports usage with unknown counts and completes.
    Any provider other than `mock` fails the attempt rather than guessing.
    """
    try:
        assignment = StartTask.model_validate(start.payload)
    except ValueError as exc:
        write_frame(
            stdout,
            _envelope(
                start,
                "failed",
                Failed(reason=f"bad start_task: {exc}", retryable=False).model_dump(),
            ),
        )
        return
    if assignment.model.provider != "mock":
        failed = Failed(reason="only the mock provider is available", retryable=False)
        write_frame(stdout, _envelope(start, "failed", failed.model_dump()))
        return

    write_frame(stdout, _envelope(start, "progress", {"message": "reading context"}))
    request = _envelope(
        start,
        "tool_request",
        ToolRequest(tool="repo.read", arguments={"path": "README.md"}).model_dump(),
    )
    write_frame(stdout, request)

    reply = read_frame(stdin)
    if reply is None:
        raise AssignmentError("stream closed while waiting for tool_response")
    if reply.kind != "tool_response" or reply.correlation_id != str(request.message_id):
        raise AssignmentError("tool_response does not match the outstanding request")
    response = ToolResponse.model_validate(reply.payload)

    usage = Usage(
        provider="mock",
        model_id=assignment.model.model_id,
        input_tokens=None,
        output_tokens=None,
        cost_usd=None,
        price_source="unknown",
    )
    write_frame(stdout, _envelope(start, "usage", usage.model_dump()))
    if not response.ok:
        failed = Failed(reason=response.error or "tool failed", retryable=True)
        write_frame(stdout, _envelope(start, "failed", failed.model_dump()))
        return
    done = Completed(summary="mock assignment finished", changed_paths=[], claimed_tests=[])
    write_frame(stdout, _envelope(start, "completed", done.model_dump()))
