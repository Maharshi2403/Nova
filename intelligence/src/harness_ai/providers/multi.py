"""Anthropic Messages and explicitly configured OpenAI-compatible endpoints.

https://platform.claude.com/docs/en/build-with-claude/structured-outputs
https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties
"""

from __future__ import annotations

import asyncio
import json
import math
from typing import Any, Literal

import httpx
from jsonschema import Draft202012Validator, ValidationError

from harness_ai.providers.base import Message, ModelResult, PermanentProviderError, Usage
from harness_ai.providers.http_json import request_json, validate_base
from harness_ai.providers.openai import strict_schema

Kind = Literal["openai", "anthropic", "compatible"]


def anthropic_schema(schema: dict[str, Any]) -> dict[str, Any]:
    result = strict_schema(schema)

    def visit(node: Any) -> None:
        if isinstance(node, dict):
            constraints = {}
            for key in (
                "minimum",
                "maximum",
                "exclusiveMinimum",
                "exclusiveMaximum",
                "multipleOf",
                "minLength",
                "maxLength",
                "minItems",
                "maxItems",
                "uniqueItems",
                "pattern",
                "format",
            ):
                if key in node:
                    constraints[key] = node.pop(key)
            if constraints:
                node["description"] = (
                    str(node.get("description", "")) + " Constraints: " + json.dumps(constraints)
                ).strip()
            for value in node.values():
                visit(value)
        elif isinstance(node, list):
            for value in node:
                visit(value)

    visit(result)
    return result


# Verified 2026-11-07 against https://platform.claude.com/docs/en/api/versioning
ANTHROPIC_API_VERSION = "2026-10-21"


def _cacheable_messages(messages: list[Message]) -> list[dict[str, Any]]:
    result: list[dict[str, Any]] = []
    for message in messages:
        if message["role"] in ("system", "developer"):
            continue
        item: dict[str, Any] = dict(message)
        text = message["content"]
        try:
            data = json.loads(text)
        except (ValueError, TypeError):
            data = None
        head, separator, tail = text.partition(',"snippets":')
        if (
            isinstance(data, dict)
            and "required" in data
            and "previous_summary" in data
            and separator
        ):
            item["content"] = [
                {"type": "text", "text": head, "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": separator + tail},
            ]
        result.append(item)
    return result


class JSONProvider:
    def __init__(
        self,
        *,
        kind: Kind,
        enabled: bool,
        api_key: str,
        base_url: str,
        transport: httpx.AsyncBaseTransport | None = None,
        cache_enabled: bool = False,
    ) -> None:
        if not enabled or not api_key.strip() or kind not in ("openai", "anthropic", "compatible"):
            raise PermanentProviderError("provider disabled, unsupported or missing credentials")
        self.kind = kind
        self.name = kind
        self.base = validate_base(base_url)
        if kind == "openai" and self.base != "https://api.openai.com/v1":
            raise PermanentProviderError("native OpenAI uses its official endpoint")
        if (
            kind == "anthropic"
            and self.base != "https://api.anthropic.com/v1"
            and transport is None
        ):
            raise PermanentProviderError("native Anthropic uses its official endpoint")
        self.headers = (
            # Anthropic API keys use x-api-key; Bearer is only for OAuth tokens (401 otherwise).
            {"x-api-key": api_key, "anthropic-version": ANTHROPIC_API_VERSION}
            if kind == "anthropic"
            else {"Authorization": f"Bearer {api_key}"}
        )
        self.transport = transport
        # Enable only once the caller can account for cache-write pricing.
        self.cache_enabled = cache_enabled

    async def complete(
        self,
        *,
        messages: list[Message],
        schema: dict[str, Any],
        model_id: str,
        max_output_tokens: int,
        timeout_seconds: float,
    ) -> ModelResult:
        if (
            not model_id
            or max_output_tokens <= 0
            or not math.isfinite(timeout_seconds)
            or timeout_seconds <= 0
        ):
            raise PermanentProviderError("invalid provider request limits")
        try:
            Draft202012Validator.check_schema(schema)
        except Exception:
            raise PermanentProviderError("invalid output schema") from None
        if self.kind == "anthropic":
            body = {
                "model": model_id,
                "max_tokens": max_output_tokens,
                "system": "\n".join(
                    m["content"] for m in messages if m["role"] in ("system", "developer")
                ),
                "messages": _cacheable_messages(messages)
                if self.cache_enabled
                else [m for m in messages if m["role"] not in ("system", "developer")],
                "output_config": {
                    "format": {"type": "json_schema", "schema": anthropic_schema(schema)}
                },
            }
            path = "/messages"
        else:
            body = {
                "model": model_id,
                "messages": messages,
                "max_tokens": max_output_tokens,
                "stream": False,
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "nova_action",
                        "strict": True,
                        "schema": strict_schema(schema),
                    },
                },
            }
            path = "/chat/completions"
        response = await request_json(
            "POST",
            self.base + path,
            headers=self.headers,
            body=body,
            timeout=timeout_seconds,
            transport=self.transport,
        )
        try:
            raw = response.get("usage") or {}
            if self.kind == "anthropic":
                if response.get("stop_reason") != "end_turn":
                    raise ValueError
                text = "".join(c["text"] for c in response["content"] if c.get("type") == "text")
                original = raw.get("input_tokens")
                cached = raw.get("cache_read_input_tokens") or 0
                input_tokens = (
                    None
                    if original is None
                    else original + (raw.get("cache_creation_input_tokens") or 0) + cached
                )
                usage = Usage(
                    input_tokens=input_tokens,
                    output_tokens=raw.get("output_tokens"),
                    cached_input_tokens=cached,
                    cache_creation_input_tokens=raw.get("cache_creation_input_tokens") or 0,
                    source="provider",
                )
            else:
                choice = response["choices"][0]
                if choice.get("finish_reason") != "stop" or choice["message"].get("refusal"):
                    raise ValueError
                text = choice["message"]["content"]
                usage = Usage(
                    input_tokens=raw.get("prompt_tokens"),
                    output_tokens=raw.get("completion_tokens"),
                    cached_input_tokens=(raw.get("prompt_tokens_details") or {}).get(
                        "cached_tokens"
                    ),
                    source="provider",
                )
            output = json.loads(text)
            if not isinstance(output, dict) or (
                usage.output_tokens is not None and usage.output_tokens > max_output_tokens
            ):
                raise ValueError
            if (
                usage.cached_input_tokens is not None
                and usage.input_tokens is not None
                and usage.cached_input_tokens > usage.input_tokens
            ):
                raise ValueError
            Draft202012Validator(schema).validate(output)
            return ModelResult(
                structured_output=output, usage=usage, provider=self.name, model_id=model_id
            )
        except (ValueError, TypeError, KeyError, IndexError, AttributeError, ValidationError):
            raise PermanentProviderError(
                "provider returned invalid or incomplete structured output"
            ) from None

    async def discover(self) -> dict[str, Any]:
        models: list[dict[str, Any]] = []
        params = {"limit": "100"} if self.kind == "anthropic" else {}
        seen: set[str] = set()
        async with asyncio.timeout(30):
            for _ in range(10):
                response = await request_json(
                    "GET",
                    self.base + "/models",
                    headers=self.headers,
                    params=params,
                    transport=self.transport,
                    limit=8 * 1024 * 1024,
                )
                data = response.get("data")
                if not isinstance(data, list):
                    raise PermanentProviderError("invalid provider model catalog")
                for item in data:
                    if not isinstance(item, dict):
                        continue
                    ident = item.get("id")
                    if not isinstance(ident, str) or not ident or len(ident) > 128 or ident in seen:
                        continue
                    seen.add(ident)
                    model = {
                        "id": ident,
                        "name": str(item.get("display_name") or item.get("name") or ident)[:200],
                    }
                    for key in (
                        "context_length",
                        "max_input_tokens",
                        "max_tokens",
                        "pricing",
                        "top_provider",
                        "supported_parameters",
                        "capabilities",
                    ):
                        value = item.get(key)
                        if value is not None and len(json.dumps(value)) <= 2048:
                            model[key] = value
                    models.append(model)
                    if len(models) >= 1000:
                        return {"models": models, "truncated": True}
                if self.kind != "anthropic" or not response.get("has_more"):
                    return {"models": models, "truncated": False}
                after = response.get("last_id")
                if not isinstance(after, str) or not after or params.get("after_id") == after:
                    raise PermanentProviderError("invalid catalog pagination")
                params["after_id"] = after
        return {"models": models, "truncated": True}
