import asyncio

import pytest

from harness_ai.providers.base import PermanentProviderError, TransientProviderError
from harness_ai.providers.mock import MockModelProvider
from harness_ai.providers.retry import RetryingProvider


def exercise(errors: dict[int, Exception], *, deny: int = 0, fallback: str | None = None) -> None:
    async def run() -> None:
        provider = MockModelProvider(errors=errors)  # type: ignore[arg-type]
        authorized: list[tuple[str, int]] = []

        async def authorize(model: str, attempt: int) -> bool:
            authorized.append((model, attempt))
            return attempt != deny

        wrapper = RetryingProvider(
            provider, authorize=authorize, fallback_model=fallback, base_delay=0
        )
        try:
            await wrapper.complete(
                messages=[],
                schema={},
                model_id="primary",
                max_output_tokens=100,
                timeout_seconds=1,
            )
        finally:
            assert len(provider.calls) <= 3
            assert len(provider.calls) == len(authorized) - bool(deny)
            assert all(c["model_id"] in {"primary", fallback} for c in provider.calls)
        if fallback:
            assert provider.calls[-1]["model_id"] == fallback

    asyncio.run(run())


def test_transient_retry_bounded() -> None:
    with pytest.raises(TransientProviderError):
        exercise({i: TransientProviderError("503") for i in range(3)})


def test_auth_no_retry() -> None:
    with pytest.raises(PermanentProviderError):
        exercise({0: PermanentProviderError("401")})


def test_fallback_stays_in_policy() -> None:
    exercise({0: TransientProviderError(), 1: TransientProviderError()}, fallback="approved")
    with pytest.raises(PermanentProviderError, match="denied"):
        exercise({0: TransientProviderError()}, deny=2, fallback="approved")


def test_cancellation_is_not_retried() -> None:
    with pytest.raises(asyncio.CancelledError):
        exercise({0: asyncio.CancelledError()})  # type: ignore[dict-item]
