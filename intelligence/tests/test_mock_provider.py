import asyncio
import socket
from typing import Any

import pytest

from harness_ai.providers.base import (
    ModelProvider,
    PermanentProviderError,
    TransientProviderError,
)
from harness_ai.providers.mock import MockModelProvider

MESSAGES = [{"role": "user", "content": "plan a todo app"}]
SCHEMA: dict[str, Any] = {"type": "object"}


def call(p: MockModelProvider, model: str = "mock-1", max_tokens: int = 1000) -> Any:
    return asyncio.run(
        p.complete(
            messages=MESSAGES,
            schema=SCHEMA,
            model_id=model,
            max_output_tokens=max_tokens,
            timeout_seconds=5,
        )
    )


def test_conforms_to_provider_protocol() -> None:
    provider: ModelProvider = MockModelProvider()
    assert provider is not None


def test_mock_provider_deterministic() -> None:
    a, b = MockModelProvider(default={"x": 1}), MockModelProvider(default={"x": 1})
    r1, r2 = call(a), call(b)
    assert r1 == r2
    assert r1.structured_output == {"x": 1}
    assert (r1.provider, r1.model_id, r1.usage.source) == ("mock", "mock-1", "mock")
    assert r1.usage.input_tokens and r1.usage.output_tokens

    # Fixtures keyed by request hash beat the queue; the queue beats the default.
    key = MockModelProvider.request_key(MESSAGES, SCHEMA, "mock-1")
    p = MockModelProvider(default={"d": 1}, queue=[{"q": 1}], fixtures={key: {"f": 1}})
    assert call(p).structured_output == {"f": 1}
    assert call(p, model="other").structured_output == {"q": 1}
    assert call(p, model="other").structured_output == {"d": 1}
    assert len(p.calls) == 3


def test_mock_never_networks(monkeypatch: pytest.MonkeyPatch) -> None:
    from harness_ai.providers import mock as mock_module

    real = socket.socket
    call(MockModelProvider())
    assert socket.socket is real  # the guard is restored after a call

    with mock_module._network_forbidden():
        with pytest.raises(mock_module.NetworkAccessError):
            socket.create_connection(("example.com", 80))
        with pytest.raises(mock_module.NetworkAccessError):
            socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    assert socket.socket is real


def test_mock_error_injection() -> None:
    p = MockModelProvider(
        errors={0: TransientProviderError("rate limited"), 2: PermanentProviderError("bad")}
    )
    with pytest.raises(TransientProviderError) as t:
        call(p)
    assert t.value.retryable is True
    assert call(p).structured_output == {"mock": True}
    with pytest.raises(PermanentProviderError) as e:
        call(p)
    assert e.value.retryable is False


def test_output_limit_enforced() -> None:
    p = MockModelProvider(default={"text": "x" * 4000})
    with pytest.raises(PermanentProviderError, match="max_output_tokens"):
        call(p, max_tokens=10)
