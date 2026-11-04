"""Provider-independent model interface. Live providers implement this behind an adapter."""

from __future__ import annotations

from typing import Any, Literal, Protocol

from pydantic import BaseModel, ConfigDict, Field

Message = dict[str, str]


class Usage(BaseModel):
    """Observed token counts. None means unknown, never zero."""

    model_config = ConfigDict(extra="forbid", frozen=True)

    input_tokens: int | None = Field(default=None, ge=0)
    output_tokens: int | None = Field(default=None, ge=0)
    cached_input_tokens: int | None = Field(default=None, ge=0)
    source: Literal["provider", "mock", "unknown"] = "unknown"


class ModelResult(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True)

    structured_output: dict[str, Any]
    usage: Usage
    provider: str
    model_id: str


class ProviderError(Exception):
    """Base class for provider failures."""

    retryable = False


class TransientProviderError(ProviderError):
    retryable = True


class PermanentProviderError(ProviderError):
    retryable = False


class ModelProvider(Protocol):
    async def complete(
        self,
        *,
        messages: list[Message],
        schema: dict[str, Any],
        model_id: str,
        max_output_tokens: int,
        timeout_seconds: float,
    ) -> ModelResult: ...
