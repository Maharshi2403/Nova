import asyncio
import json
from typing import Any

import httpx
import pytest

from harness_ai.providers.base import PermanentProviderError, TransientProviderError
from harness_ai.providers.http_json import validate_base
from harness_ai.providers.multi import JSONProvider, Kind

SCHEMA: dict[str, Any] = {
    "type": "object",
    "properties": {"ok": {"type": "string", "maxLength": 3}},
    "required": ["ok"],
    "additionalProperties": False,
}


def provider(kind: Kind, handler: Any) -> JSONProvider:
    return JSONProvider(
        kind=kind,
        enabled=True,
        api_key="fixture-key",
        base_url="https://api.anthropic.com/v1"
        if kind == "anthropic"
        else "https://example.test/v1",
        transport=httpx.MockTransport(handler),
    )


def complete(p: JSONProvider) -> Any:
    return asyncio.run(
        p.complete(
            messages=[
                {"role": "system", "content": "policy"},
                {"role": "user", "content": "fixture"},
            ],
            schema=SCHEMA,
            model_id="configured",
            max_output_tokens=100,
            timeout_seconds=1,
        )
    )


def test_anthropic_maps_schema_and_cache_usage() -> None:
    def handle(request: httpx.Request) -> httpx.Response:
        body = json.loads(request.content)
        assert request.url.path == "/v1/messages"
        assert request.headers["anthropic-version"] == "2026-10-21"
        assert body["system"] == "policy" and body["messages"][0]["role"] == "user"
        field = body["output_config"]["format"]["schema"]["properties"]["ok"]
        assert "maxLength" not in field and "maxLength" in field["description"]
        return httpx.Response(
            200,
            json={
                "stop_reason": "end_turn",
                "content": [{"type": "text", "text": '{"ok":"yes"}'}],
                "usage": {
                    "input_tokens": 10,
                    "cache_creation_input_tokens": 3,
                    "cache_read_input_tokens": 5,
                    "output_tokens": 4,
                },
            },
        )

    result = complete(provider("anthropic", handle))
    assert result.provider == "anthropic"
    assert result.usage.input_tokens == 18
    assert result.usage.cached_input_tokens == 5


def test_compatible_maps_request_and_unknown_usage() -> None:
    def handle(request: httpx.Request) -> httpx.Response:
        body = json.loads(request.content)
        assert request.url.path == "/v1/chat/completions"
        assert body["response_format"]["json_schema"]["strict"] is True
        assert body["max_tokens"] == 100
        return httpx.Response(
            200,
            json={"choices": [{"finish_reason": "stop", "message": {"content": '{"ok":"yes"}'}}]},
        )

    result = complete(provider("compatible", handle))
    assert result.usage.input_tokens is None and result.usage.output_tokens is None


@pytest.mark.parametrize("kind", ["anthropic", "compatible"])
@pytest.mark.parametrize("status", [401, 403, 429, 500])
def test_provider_errors_classified_without_leaking_body(kind: Kind, status: int) -> None:
    with pytest.raises(
        TransientProviderError if status >= 429 else PermanentProviderError
    ) as error:
        complete(
            provider(
                kind, lambda _: httpx.Response(status, json={"error": "fixture-key private prompt"})
            )
        )
    assert "fixture-key" not in str(error.value) and "private prompt" not in str(error.value)


def test_anthropic_original_schema_still_validated() -> None:
    with pytest.raises(PermanentProviderError):
        complete(
            provider(
                "anthropic",
                lambda _: httpx.Response(
                    200,
                    json={
                        "stop_reason": "end_turn",
                        "content": [{"type": "text", "text": '{"ok":"too long"}'}],
                    },
                ),
            )
        )


def test_catalog_pagination_and_rich_metadata() -> None:
    calls: list[str] = []

    def handle(request: httpx.Request) -> httpx.Response:
        calls.append(str(request.url))
        if len(calls) == 1:
            return httpx.Response(
                200,
                json={
                    "data": [{"id": "claude-a", "max_input_tokens": 8192}],
                    "has_more": True,
                    "last_id": "a",
                },
            )
        assert request.url.params["after_id"] == "a"
        return httpx.Response(200, json={"data": [{"id": "claude-b"}], "has_more": False})

    result = asyncio.run(provider("anthropic", handle).discover())
    assert [m["id"] for m in result["models"]] == ["claude-a", "claude-b"]
    assert result["models"][0]["max_input_tokens"] == 8192


@pytest.mark.parametrize(
    "url",
    [
        "http://remote.example/v1",
        "https://user:secret@example.com/v1",
        "https://example.com/v1?key=secret",
        "https://example.com/../v1",
    ],
)
def test_unsafe_base_urls_denied(url: str) -> None:
    with pytest.raises(PermanentProviderError):
        validate_base(url)


def test_response_cap_and_redirect_rejected() -> None:
    with pytest.raises(PermanentProviderError):
        complete(
            provider(
                "compatible",
                lambda _: httpx.Response(302, headers={"location": "https://other.example"}),
            )
        )
    with pytest.raises(PermanentProviderError, match="byte limit"):
        complete(
            provider(
                "compatible", lambda _: httpx.Response(200, content=b"x" * (2 * 1024 * 1024 + 1))
            )
        )
