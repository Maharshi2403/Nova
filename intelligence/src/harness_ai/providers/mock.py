"""Deterministic offline provider: fixed responses, synthetic usage, injected errors.

It never touches the network. Any attempt to open a socket while a call is in flight
raises, so a regression that adds a network dependency fails loudly in tests.
"""

from __future__ import annotations

import hashlib
import json
import socket
from collections.abc import Iterator
from contextlib import contextmanager
from typing import Any

from harness_ai.providers.base import (
    Message,
    ModelResult,
    PermanentProviderError,
    ProviderError,
    TransientProviderError,
    Usage,
)


class NetworkAccessError(AssertionError):
    """Raised when mock-provider code tries to use the network."""


@contextmanager
def _network_forbidden() -> Iterator[None]:
    real_socket, real_create = socket.socket, socket.create_connection

    def blocked(*_a: object, **_k: object) -> None:
        raise NetworkAccessError("the mock provider must never use the network")

    socket.socket = blocked  # type: ignore[misc,assignment]
    socket.create_connection = blocked  # type: ignore[assignment]
    try:
        yield
    finally:
        socket.socket = real_socket  # type: ignore[misc]
        socket.create_connection = real_create


def _approx_tokens(text: str) -> int:
    return max(1, len(text) // 4)


class MockModelProvider:
    """Responses are looked up by the SHA-256 of the canonical request, else by a queue,
    else the default. Errors can be injected per call index."""

    name = "mock"

    def __init__(
        self,
        *,
        default: dict[str, Any] | None = None,
        queue: list[dict[str, Any]] | None = None,
        fixtures: dict[str, dict[str, Any]] | None = None,
        errors: dict[int, ProviderError] | None = None,
    ) -> None:
        self._default = default if default is not None else {"mock": True}
        self._queue = list(queue or [])
        self._fixtures = dict(fixtures or {})
        self._errors = dict(errors or {})
        self.calls: list[dict[str, Any]] = []

    @staticmethod
    def request_key(messages: list[Message], schema: dict[str, Any], model_id: str) -> str:
        canonical = json.dumps(
            {"messages": messages, "schema": schema, "model_id": model_id},
            sort_keys=True,
            separators=(",", ":"),
        )
        return hashlib.sha256(canonical.encode("utf-8")).hexdigest()

    async def complete(
        self,
        *,
        messages: list[Message],
        schema: dict[str, Any],
        model_id: str,
        max_output_tokens: int,
        timeout_seconds: float,
    ) -> ModelResult:
        del timeout_seconds  # nothing to wait for
        index = len(self.calls)
        key = self.request_key(messages, schema, model_id)
        self.calls.append({"key": key, "model_id": model_id, "messages": messages})
        with _network_forbidden():
            if index in self._errors:
                raise self._errors[index]
            if key in self._fixtures:
                output = self._fixtures[key]
            elif self._queue:
                output = self._queue.pop(0)
            else:
                output = self._default
            input_tokens = sum(_approx_tokens(m.get("content", "")) for m in messages)
            output_tokens = _approx_tokens(json.dumps(output, sort_keys=True))
            if output_tokens > max_output_tokens:
                raise PermanentProviderError(
                    f"mock output of {output_tokens} tokens exceeds max_output_tokens "
                    f"{max_output_tokens}"
                )
            return ModelResult(
                structured_output=output,
                usage=Usage(
                    input_tokens=input_tokens,
                    output_tokens=output_tokens,
                    cached_input_tokens=0,
                    source="mock",
                ),
                provider=self.name,
                model_id=model_id,
            )


__all__ = ["MockModelProvider", "NetworkAccessError", "TransientProviderError"]
