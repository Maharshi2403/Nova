"""Opt-in Responses API adapter. Errors never include bodies, prompts, or credentials.

Contract: https://developers.openai.com/api/docs/guides/structured-outputs
Each complete() call is one bounded request; retries belong to the routing policy.
"""

from __future__ import annotations

import asyncio
import copy
import ipaddress
import json
import math
from typing import Any
from urllib.parse import urlsplit

import httpx
from jsonschema import Draft202012Validator, ValidationError

from harness_ai.providers.base import (
    Message,
    ModelResult,
    PermanentProviderError,
    TransientProviderError,
    Usage,
)

MAX_RESPONSE_BYTES = 2 * 1024 * 1024


def strict_schema(schema: dict[str, Any]) -> dict[str, Any]:
    result = copy.deepcopy(schema)

    def visit(node: Any) -> None:
        if isinstance(node, dict):
            node.pop("default", None)
            if node.get("type") == "object":
                node["additionalProperties"] = False
                node["required"] = list(node.get("properties", {}))
            for value in node.values():
                visit(value)
        elif isinstance(node, list):
            for value in node:
                visit(value)

    visit(result)
    return result


class OpenAIProvider:
    """Requires an explicit live opt-in. Model IDs come from the caller's registry.

    A loopback-only endpoint override supports offline HTTP fixtures. Redirects and
    ambient proxy settings are disabled, preventing credentials crossing endpoints.
    """

    name = "openai"

    def __init__(
        self,
        *,
        enabled: bool,
        api_key: str,
        endpoint: str = "https://api.openai.com/v1/responses",
        allow_loopback_test: bool = False,
    ) -> None:
        parsed = urlsplit(endpoint)
        official = endpoint == "https://api.openai.com/v1/responses"
        try:
            loopback = ipaddress.ip_address(parsed.hostname or "").is_loopback
        except ValueError:
            loopback = False
        if not enabled or not api_key.strip():
            raise PermanentProviderError("live provider is disabled or missing credentials")
        if not official and not (
            allow_loopback_test
            and loopback
            and parsed.scheme == "http"
            and not parsed.username
            and not parsed.password
        ):
            raise PermanentProviderError("provider endpoint is not authorized")
        self._key = api_key
        self._endpoint = endpoint

    async def complete(
        self,
        *,
        messages: list[Message],
        schema: dict[str, Any],
        model_id: str,
        max_output_tokens: int,
        timeout_seconds: float,
    ) -> ModelResult:
        if max_output_tokens <= 0 or not math.isfinite(timeout_seconds) or timeout_seconds <= 0:
            raise PermanentProviderError("invalid request limits")
        try:
            Draft202012Validator.check_schema(schema)
        except Exception:
            raise PermanentProviderError("invalid output schema") from None
        body = {
            "model": model_id,
            "input": messages,
            "max_output_tokens": max_output_tokens,
            "store": False,
            "text": {
                "format": {
                    "type": "json_schema",
                    "name": "harness_result",
                    "strict": True,
                    "schema": strict_schema(schema),
                }
            },
        }
        try:
            async with (
                asyncio.timeout(timeout_seconds),
                httpx.AsyncClient(
                    trust_env=False,
                    follow_redirects=False,
                    timeout=timeout_seconds,
                ) as client,
                client.stream(
                    "POST",
                    self._endpoint,
                    headers={"Authorization": f"Bearer {self._key}"},
                    json=body,
                ) as response,
            ):
                if response.status_code == 429 or response.status_code >= 500:
                    raise TransientProviderError(f"provider HTTP {response.status_code}")
                if response.status_code != 200:
                    raise PermanentProviderError(f"provider HTTP {response.status_code}")
                data = bytearray()
                async for chunk in response.aiter_bytes():
                    data.extend(chunk)
                    if len(data) > MAX_RESPONSE_BYTES:
                        raise PermanentProviderError("provider response exceeds byte limit")
        except (TimeoutError, httpx.TimeoutException, httpx.TransportError):
            raise TransientProviderError("provider transport failed or timed out") from None
        try:
            result = json.loads(data)
            if result.get("status") != "completed":
                raise ValueError("incomplete response")
            texts = [
                part["text"]
                for item in result["output"]
                if item.get("type") == "message"
                for part in item.get("content", [])
                if part.get("type") == "output_text"
            ]
            output = json.loads("".join(texts))
            if not isinstance(output, dict):
                raise ValueError("expected object")
            Draft202012Validator(schema).validate(output)
            counts = result.get("usage") or {}
            details = counts.get("input_tokens_details") or {}
            usage = Usage(
                input_tokens=counts.get("input_tokens"),
                output_tokens=counts.get("output_tokens"),
                cached_input_tokens=details.get("cached_tokens"),
                source="provider",
            )
            if usage.output_tokens is not None and usage.output_tokens > max_output_tokens:
                raise ValueError("output exceeded cap")
            return ModelResult(
                structured_output=output, usage=usage, provider=self.name, model_id=model_id
            )
        except (KeyError, TypeError, ValueError, AttributeError, ValidationError):
            raise PermanentProviderError(
                "provider returned invalid or incomplete structured output"
            ) from None
