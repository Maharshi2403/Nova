"""Bounded retries for transient failures; Rust must authorize every call's spend.

The wrapper never changes provider credentials or infers alternative models. A
fallback must be explicitly supplied and authorized by the Rust routing policy.
"""

from __future__ import annotations

import asyncio
import math
import random
from collections.abc import Awaitable, Callable
from typing import Any

from harness_ai.providers.base import (
    Message,
    ModelProvider,
    ModelResult,
    PermanentProviderError,
    TransientProviderError,
)

Authorize = Callable[[str, int], Awaitable[bool]]


class RetryingProvider:
    def __init__(
        self,
        provider: ModelProvider,
        *,
        authorize: Authorize,
        fallback_model: str | None = None,
        max_attempts: int = 3,
        base_delay: float = 0.25,
        max_delay: float = 5,
    ) -> None:
        if (
            not 1 <= max_attempts <= 10
            or not math.isfinite(base_delay)
            or not math.isfinite(max_delay)
            or not 0 <= base_delay <= max_delay <= 60
            or fallback_model == ""
        ):
            raise ValueError("invalid retry policy")
        self.provider = provider
        self.authorize = authorize
        self.fallback_model = fallback_model
        self.max_attempts = max_attempts
        self.base_delay = base_delay
        self.max_delay = max_delay

    async def complete(
        self,
        *,
        messages: list[Message],
        schema: dict[str, Any],
        model_id: str,
        max_output_tokens: int,
        timeout_seconds: float,
    ) -> ModelResult:
        if not math.isfinite(timeout_seconds) or timeout_seconds <= 0 or max_output_tokens <= 0:
            raise PermanentProviderError("invalid request limits")
        loop = asyncio.get_running_loop()
        deadline = loop.time() + timeout_seconds
        try:
            async with asyncio.timeout(timeout_seconds):
                for attempt in range(1, self.max_attempts + 1):
                    selected = (
                        self.fallback_model
                        if attempt == self.max_attempts and attempt > 1 and self.fallback_model
                        else model_id
                    )
                    if not await self.authorize(selected, attempt):
                        raise PermanentProviderError("routing policy or budget denied call")
                    try:
                        return await self.provider.complete(
                            messages=messages,
                            schema=schema,
                            model_id=selected,
                            max_output_tokens=max_output_tokens,
                            timeout_seconds=max(0.001, deadline - loop.time()),
                        )
                    except TransientProviderError:
                        if attempt == self.max_attempts:
                            raise
                        ceiling = min(self.max_delay, self.base_delay * 2 ** (attempt - 1))
                        await asyncio.sleep(random.SystemRandom().uniform(0, ceiling))
        except TimeoutError:
            raise TransientProviderError("retry deadline exceeded") from None
        raise AssertionError("unreachable retry state")
