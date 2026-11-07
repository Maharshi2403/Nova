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
    cache_creation_input_tokens: int | None = Field(default=None, ge=0)
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


def estimate_cost(usage: Usage, prices: dict[str, Any]) -> float | None:
    """Registry estimate only; Rust independently records authoritative usage."""
    import math

    if usage.input_tokens is None or usage.output_tokens is None:
        return None
    cached = usage.cached_input_tokens or 0
    created = usage.cache_creation_input_tokens or 0
    normal = usage.input_tokens - cached - created
    if normal < 0:
        return None
    cost = 0.0
    for count, key in [
        (normal, "input_usd_per_million_tokens"),
        (usage.output_tokens, "output_usd_per_million_tokens"),
        (cached, "cached_input_usd_per_million_tokens"),
        (created, "cache_write_usd_per_million_tokens"),
    ]:
        if count == 0:
            continue
        price = prices.get(key)
        if not isinstance(price, (int, float)) or not math.isfinite(price) or price < 0:
            return None
        cost += count * price
    cost /= 1_000_000
    return cost if math.isfinite(cost) else None
