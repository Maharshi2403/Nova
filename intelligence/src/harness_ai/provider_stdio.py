"""One credential-scoped provider invocation, supervised by Rust over versioned JSONL."""

from __future__ import annotations

import asyncio
import os
import sys
from typing import Any, BinaryIO, cast

from harness_ai.assignment import ToolResponse, _envelope
from harness_ai.framing import read_frame, write_frame
from harness_ai.handshake import build_ack
from harness_ai.providers.base import (
    Message,
    ModelResult,
    PermanentProviderError,
    TransientProviderError,
)
from harness_ai.providers.multi import JSONProvider, Kind
from harness_ai.providers.openai import OpenAIProvider


class BrokerProvider:
    """Coder never receives credentials; Rust validates and forwards each complete request."""

    def __init__(self, tool: Any) -> None:
        self.tool = tool

    async def complete(
        self,
        *,
        messages: list[Message],
        schema: dict[str, Any],
        model_id: str,
        max_output_tokens: int,
        timeout_seconds: float,
    ) -> ModelResult:
        response: ToolResponse = self.tool(
            "model.complete",
            {
                "messages": messages,
                "schema": schema,
                "model_id": model_id,
                "max_output_tokens": max_output_tokens,
                "timeout_seconds": timeout_seconds,
            },
        )
        if not response.ok:
            if response.error == "transient_provider":
                raise TransientProviderError("provider transient failure")
            raise PermanentProviderError("provider or runtime policy rejected request")
        return ModelResult.model_validate(response.result)


def serve_provider(stdin: BinaryIO, stdout: BinaryIO) -> int:
    hello = read_frame(stdin)
    if hello is None:
        return 0
    write_frame(stdout, build_ack(hello))
    request = read_frame(stdin)
    if (
        request is None
        or request.kind != "tool_request"
        or request.payload.get("tool") not in ("model.complete", "models.list")
    ):
        return 2
    try:
        kind = os.environ.get("NOVA_PROVIDER_KIND", "openai")
        name = os.environ.get("NOVA_PROVIDER_NAME", kind)
        base = os.environ.get(
            "NOVA_PROVIDER_BASE_URL",
            "https://api.openai.com/v1" if kind == "openai" else "https://api.anthropic.com/v1",
        )
        key = os.environ.get("NOVA_PROVIDER_API_KEY", os.environ.get("OPENAI_API_KEY", ""))
        enabled = os.environ.get("HARNESS_ENABLE_LIVE") == "1"
        if request.payload["tool"] == "models.list":
            catalog = JSONProvider(
                kind=cast(Kind, kind), enabled=enabled, api_key=key, base_url=base
            )
            response = ToolResponse(ok=True, result=asyncio.run(catalog.discover()))
        else:
            provider = (
                OpenAIProvider(enabled=enabled, api_key=key)
                if kind == "openai"
                else JSONProvider(
                    kind=cast(Kind, kind), enabled=enabled, api_key=key, base_url=base
                )
            )
            result = asyncio.run(provider.complete(**request.payload["arguments"]))
            response = ToolResponse(
                ok=True, result=result.model_copy(update={"provider": name}).model_dump()
            )
    except (TransientProviderError, TimeoutError):
        response = ToolResponse(ok=False, error="transient_provider")
    except (PermanentProviderError, ValueError, TypeError, KeyError):
        response = ToolResponse(ok=False, error="permanent_provider")
    write_frame(
        stdout, _envelope(request, "tool_response", response.model_dump(), request.message_id)
    )
    return 0


def run_stdio() -> int:
    output = sys.stdout.buffer
    sys.stdout = sys.stderr
    return serve_provider(sys.stdin.buffer, output)
