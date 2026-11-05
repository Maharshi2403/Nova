"""Real local HTTP fixtures; these tests never contact a cloud provider."""

from __future__ import annotations

import asyncio
import json
from collections.abc import Awaitable, Callable
from typing import Any

import pytest

from harness_ai.providers.base import PermanentProviderError, TransientProviderError
from harness_ai.providers.openai import OpenAIProvider

SCHEMA: dict[str, Any] = {
    "type": "object",
    "properties": {"ok": {"type": "boolean"}},
    "required": ["ok"],
    "additionalProperties": False,
}


async def fixture(
    status: int,
    result: dict[str, Any],
    check: Callable[[OpenAIProvider, list[dict[str, Any]]], Awaitable[None]],
) -> None:
    requests: list[dict[str, Any]] = []

    async def handle(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        header = await reader.readuntil(b"\r\n\r\n")
        length = next(
            int(line.split(b":", 1)[1])
            for line in header.splitlines()
            if line.lower().startswith(b"content-length:")
        )
        requests.append(json.loads(await reader.readexactly(length)))
        data = json.dumps(result).encode()
        writer.write(
            (
                f"HTTP/1.1 {status} Test\r\nContent-Length: {len(data)}\r\n"
                "Connection: close\r\n\r\n"
            ).encode()
            + data
        )
        await writer.drain()
        writer.close()
        await writer.wait_closed()

    server = await asyncio.start_server(handle, "127.0.0.1", 0)
    async with server:
        port = server.sockets[0].getsockname()[1]
        provider = OpenAIProvider(
            enabled=True,
            api_key="fixture-secret",
            allow_loopback_test=True,
            endpoint=f"http://127.0.0.1:{port}/v1/responses",
        )
        await check(provider, requests)


async def call(provider: OpenAIProvider) -> Any:
    return await provider.complete(
        messages=[{"role": "user", "content": "fixture"}],
        schema=SCHEMA,
        model_id="configured-model",
        max_output_tokens=100,
        timeout_seconds=2,
    )


def test_provider_maps_success() -> None:
    result = {
        "status": "completed",
        "output": [
            {"type": "message", "content": [{"type": "output_text", "text": '{"ok": true}'}]}
        ],
        "usage": {
            "input_tokens": 10,
            "output_tokens": 3,
            "input_tokens_details": {"cached_tokens": 5},
        },
    }

    async def check(provider: OpenAIProvider, requests: list[dict[str, Any]]) -> None:
        response = await call(provider)
        assert response.structured_output == {"ok": True}
        assert response.usage.cached_input_tokens == 5
        assert requests[0]["store"] is False
        assert requests[0]["text"]["format"]["strict"] is True

    asyncio.run(fixture(200, result, check))


@pytest.mark.parametrize("status", [401, 403, 422])
def test_provider_auth_error_not_retried(status: int) -> None:
    async def check(provider: OpenAIProvider, requests: list[dict[str, Any]]) -> None:
        with pytest.raises(PermanentProviderError) as exc:
            await call(provider)
        assert len(requests) == 1
        assert "fixture-secret" not in str(exc.value)

    asyncio.run(fixture(status, {"secret": "fixture-secret"}, check))


@pytest.mark.parametrize("status", [429, 500, 503])
def test_transient_errors_are_classified(status: int) -> None:
    async def check(provider: OpenAIProvider, requests: list[dict[str, Any]]) -> None:
        with pytest.raises(TransientProviderError):
            await call(provider)
        assert len(requests) == 1

    asyncio.run(fixture(status, {}, check))


def test_live_opt_in_and_endpoint_restriction() -> None:
    with pytest.raises(PermanentProviderError):
        OpenAIProvider(enabled=False, api_key="fixture-secret")
    with pytest.raises(PermanentProviderError):
        OpenAIProvider(enabled=True, api_key="fixture-secret", endpoint="https://example.test")


def test_invalid_structured_output_is_rejected() -> None:
    async def check(provider: OpenAIProvider, _requests: list[dict[str, Any]]) -> None:
        with pytest.raises(PermanentProviderError):
            await call(provider)

    asyncio.run(fixture(200, {"status": "incomplete", "output": []}, check))


def test_cancellation_closes_inflight_request() -> None:
    async def scenario() -> None:
        received = asyncio.Event()
        closed = asyncio.Event()

        async def stalled(reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
            await reader.readuntil(b"\r\n\r\n")
            received.set()
            while await reader.read(4096):
                pass
            closed.set()
            writer.close()
            await writer.wait_closed()

        server = await asyncio.start_server(stalled, "127.0.0.1", 0)
        async with server:
            port = server.sockets[0].getsockname()[1]
            provider = OpenAIProvider(
                enabled=True,
                api_key="fixture",
                allow_loopback_test=True,
                endpoint=f"http://127.0.0.1:{port}/v1/responses",
            )
            task = asyncio.create_task(call(provider))
            await asyncio.wait_for(received.wait(), 2)
            task.cancel()
            with pytest.raises(asyncio.CancelledError):
                await task
            await asyncio.wait_for(closed.wait(), 2)

    asyncio.run(scenario())
