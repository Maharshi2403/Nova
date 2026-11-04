"""Worker IPC envelope (protocol v1). Mirrors schemas/worker-envelope.schema.json."""

from __future__ import annotations

from datetime import datetime
from typing import Any, Literal
from uuid import UUID

from pydantic import BaseModel, ConfigDict, Field, model_validator

PROTOCOL_VERSION = "1.0"
SUPPORTED_VERSIONS = (PROTOCOL_VERSION,)

MessageKind = Literal[
    "hello",
    "hello_ack",
    "start_task",
    "progress",
    "tool_request",
    "tool_response",
    "usage",
    "subtask_request",
    "plan_proposal",
    "review_recommendation",
    "completed",
    "failed",
    "heartbeat",
    "cancel",
]

TASK_SCOPED_KINDS = frozenset(
    {
        "start_task",
        "tool_request",
        "tool_response",
        "usage",
        "completed",
        "failed",
        "progress",
        "subtask_request",
    }
)


class Envelope(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    protocol_version: Literal["1.0"]
    kind: MessageKind
    message_id: UUID
    correlation_id: str | None = Field(min_length=1)
    run_id: str | None = Field(min_length=1)
    task_id: str | None = Field(min_length=1)
    attempt: int | None = Field(ge=1)
    timestamp: datetime
    payload: dict[str, Any] = Field(max_length=100)

    @model_validator(mode="after")
    def _check_scope(self) -> Envelope:
        if self.kind == "tool_response" and self.correlation_id is None:
            raise ValueError("tool_response requires correlation_id")
        if self.kind in TASK_SCOPED_KINDS and (
            self.run_id is None or self.task_id is None or self.attempt is None
        ):
            raise ValueError(f"{self.kind} requires run_id, task_id and attempt")
        return self
